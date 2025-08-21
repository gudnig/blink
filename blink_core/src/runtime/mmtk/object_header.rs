// blink_core/src/runtime/mmtk/object_header.rs
use mmtk::{
    util::{
        metadata::side_metadata::{SideMetadataOffset, SideMetadataSpec, GLOBAL_SIDE_METADATA_VM_BASE_OFFSET}, Address, ObjectReference
    }, 
    vm::ObjectModel
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use parking_lot::{Condvar as PLCondvar, Mutex as PLMutex, Once};


use crate::runtime::{BlinkObjectModel, TypeTag};

#[repr(C)]
pub struct ObjectHeader {
    // Word 0: reserved for GC
    pub gc_metadata: usize,

    // Word 1: your VM metadata
    pub type_tag: i8,
    pub _padding1: [u8; 3],
    pub total_size: u32,

    // Word 2: synchronization mark word (thin/inflated/hash/etc.)
    pub lockword: core::sync::atomic::AtomicU64, 
}

impl ObjectHeader {
    pub const SIZE: usize = std::mem::size_of::<ObjectHeader>();
    
    pub fn new(type_tag: TypeTag, data_size: usize) -> Self {
        Self {
            gc_metadata: 0,  // Initially zero - GC will manage this
            type_tag: type_tag as i8,
            _padding1: [0; 3],
            total_size: (Self::SIZE + data_size) as u32,
            lockword: core::sync::atomic::AtomicU64::new(0), // UNLOCKED
        }
    }
    
    #[inline]
    pub fn get_type(&self) -> TypeTag {
        unsafe { std::mem::transmute(self.type_tag) }
    }
}

// === Side Metadata Specification for Object IDs ===

/// Object ID side metadata - 64 bits per object, global scope
/// This will be placed at the VM base address automatically by MMTk
pub const OBJ_ID_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    name: "blink.obj_id",
    is_global: true,  // Global so it survives across all spaces
    // Use the VM base offset - MMTk will place it after core metadata
    offset: GLOBAL_SIDE_METADATA_VM_BASE_OFFSET,
    log_num_of_bits: 6,  // 2^6 = 64 bits per entry
    log_bytes_in_region: 3,  // 2^3 = 8 bytes per object (min object size)
};

// === Lock State Definitions ===

#[derive(Copy, Clone, Eq, PartialEq)]
#[repr(u16)]
enum LState { Unlocked=0, Thin=1, Inflated=2, Hashed=3 }

#[inline] fn lw_state(x:u64)->LState { unsafe { core::mem::transmute((x & 0xFFFF) as u16) } }
#[inline] fn lw_ver(x:u64)->u64 { (x >> 56) & 0x7F }
#[inline] fn lw_payload(x:u64)->u64 { (x >> 16) & 0xFFFF_FFFFFF }
#[inline] fn lw_compose(ver:u64, payload:u64, st:LState)->u64 {
    ((ver & 0x7F) << 56) | ((payload & 0xFFFF_FFFFFF) << 16) | (st as u64)
}

// Thin payload: [ rec:8 | owner_id:32 ] packed into 40 bits
#[inline] fn thin_payload(owner_id:u32, rec:u8)->u64 {
    ((owner_id as u64) & 0xFFFF_FFFF) | ((rec as u64) << 32)
}
#[inline] fn thin_owner(p:u64)->u32 { (p & 0xFFFF_FFFF) as u32 }
#[inline] fn thin_rec(p:u64)->u8 { (p >> 32) as u8 }

// === Object ID and Monitor Handle ===

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ObjId(pub u64);

#[derive(Copy, Clone)]
struct MonHandle(u64); // Monitor handle that fits in lockword payload

static OBJ_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Get or create object ID for an object using side metadata
pub fn obj_id(obj: ObjectReference) -> ObjId {
    // Try to load existing ID
    let existing_id = OBJ_ID_METADATA_SPEC.load_atomic::<u64>(obj.to_raw_address(), Ordering::Acquire);
    
    if existing_id != 0 {
        return ObjId(existing_id);
    }
    
    // Create new ID
    let new_id = OBJ_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    
    // Try to store it (atomic compare-exchange to handle race conditions)
    match OBJ_ID_METADATA_SPEC.compare_exchange_atomic::<u64>(
        obj.to_raw_address(), 
        0,  // expected (uninitialized)
        new_id,  // new value
        Ordering::Release, 
        Ordering::Acquire
    ) {
        Ok(_) => ObjId(new_id),  // We won the race
        Err(actual) => ObjId(actual),  // Someone else won, use their ID
    }
}

/// Store object ID when object is allocated (called from allocation path)
pub fn store_obj_id(obj: ObjectReference, id: ObjId) {
    OBJ_ID_METADATA_SPEC.store_atomic::<u64>(obj.to_raw_address(), id.0, Ordering::Release);
}

/// Generate a new unique object ID (for allocation)
pub fn new_obj_id() -> ObjId {
    ObjId(OBJ_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
}

// === Helper Functions ===

#[inline]
fn header_ptr(obj: ObjectReference) -> *mut ObjectHeader {
    (BlinkObjectModel::ref_to_header(obj)).to_mut_ptr::<ObjectHeader>()
}

#[inline]
pub unsafe fn header_ref(obj: ObjectReference) -> &'static ObjectHeader {
    &*header_ptr(obj)
}

#[inline]
pub unsafe fn header_mut(obj: ObjectReference) -> &'static mut ObjectHeader {
    &mut *header_ptr(obj)
}

#[inline]
fn lockword(obj: ObjectReference) -> &'static core::sync::atomic::AtomicU64 {
    unsafe { &(*header_ptr(obj)).lockword }
}

// === Monitor System ===

#[derive(Debug)]
pub struct Monitor {
    mutex: PLMutex<MonitorState>,
    condvar: PLCondvar,
}

#[derive(Debug)]
struct MonitorState {
    owner: Option<u32>,  // Current owner goroutine ID
    recursion_count: u32,
    waiting_queue: Vec<u32>, // Goroutines waiting to acquire
}

impl Monitor {
    fn new() -> Self {
        Self {
            mutex: PLMutex::new(MonitorState {
                owner: None,
                recursion_count: 0,
                waiting_queue: Vec::new(),
            }),
            condvar: PLCondvar::new(),
        }
    }
}

// === Monitor Registry ===

struct Monitors {
    registry: PLMutex<HashMap<u64, Arc<Monitor>>>,
}

impl Monitors {
    fn new() -> Self {
        Self {
            registry: PLMutex::new(HashMap::new()),
        }
    }

    fn get_or_create(&self, obj_id: ObjId) -> MonHandle {
        let mut registry = self.registry.lock();
        
        let monitor_id = obj_id.0;
        
        if !registry.contains_key(&monitor_id) {
            registry.insert(monitor_id, Arc::new(Monitor::new()));
        }
        
        MonHandle(monitor_id)
    }

    fn get(&self, handle: MonHandle) -> Arc<Monitor> {
        let registry = self.registry.lock();
        registry.get(&handle.0)
            .expect("Monitor should exist")
            .clone()
    }

}

static MONITORS_ONCE: Once = Once::new();
static mut MONITORS: Option<Monitors> = None;

fn get_monitors() -> &'static Monitors {
    unsafe {
        MONITORS_ONCE.call_once(|| {
            MONITORS = Some(Monitors::new());
        });
        MONITORS.as_ref().unwrap()
    }
}


// === Goroutine ID Management ===

static GOROUTINE_COUNTER: AtomicU32 = AtomicU32::new(1);

thread_local! {
    static CURRENT_GOROUTINE_ID: std::cell::Cell<Option<u32>> = std::cell::Cell::new(None);
}

fn thread_id_u32() -> u32 {
    CURRENT_GOROUTINE_ID.with(|id| {
        if let Some(existing) = id.get() {
            existing
        } else {
            let new_id = GOROUTINE_COUNTER.fetch_add(1, Ordering::Relaxed);
            id.set(Some(new_id));
            new_id
        }
    })
}

pub fn set_current_goroutine_id(goroutine_id: u32) {
    CURRENT_GOROUTINE_ID.with(|id| id.set(Some(goroutine_id)));
}

pub fn get_current_goroutine_id() -> u32 {
    thread_id_u32()
}

// === Monitor Operations ===

fn monitor_lock(monitor: &Monitor, goroutine_id: u32) {
    let mut state = monitor.mutex.lock();
    
    // If already owned by this goroutine, just increment recursion
    if state.owner == Some(goroutine_id) {
        state.recursion_count += 1;
        return;
    }
    
    // Wait until monitor is available
    while state.owner.is_some() {
        state.waiting_queue.push(goroutine_id);
        monitor.condvar.wait(&mut state);
        
        // Remove from waiting queue when woken up
        state.waiting_queue.retain(|&id| id != goroutine_id);
    }
    
    // Acquire the monitor
    state.owner = Some(goroutine_id);
    state.recursion_count = 1;
}

fn monitor_unlock(monitor: &Monitor, goroutine_id: u32) {
    let mut state = monitor.mutex.lock();
    
    debug_assert_eq!(state.owner, Some(goroutine_id), "Unlocking monitor not owned by this goroutine");
    
    state.recursion_count -= 1;
    
    if state.recursion_count == 0 {
        state.owner = None;
        // Wake up one waiting goroutine
        if !state.waiting_queue.is_empty() {
            monitor.condvar.notify_one();
        }
    }
}

// === Inflation Functions ===

fn inflate_and_lock(obj: ObjectReference, lw: &core::sync::atomic::AtomicU64, me: u32) {
    let id = obj_id(obj);
    let h = get_monitors().get_or_create(id);

    // Try to flag header as INFLATED(handle)
    loop {
        let cur = lw.load(core::sync::atomic::Ordering::Acquire);
        if lw_state(cur) == LState::Inflated { 
            break; 
        }
        let next = lw_compose(lw_ver(cur).wrapping_add(1), h.0, LState::Inflated);
        if lw.compare_exchange_weak(cur, next,
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire).is_ok() { 
            break; 
        }
    }
    inflated_lock(obj, lw, me)
}

fn inflated_lock(_obj: ObjectReference, lw: &core::sync::atomic::AtomicU64, me: u32) {
    let h = MonHandle(lw_payload(lw.load(core::sync::atomic::Ordering::Acquire)));
    let m = get_monitors().get(h);
    monitor_lock(&m, me);
}

fn inflated_unlock(_obj: ObjectReference, lw: &core::sync::atomic::AtomicU64, me: u32) {
    let h = MonHandle(lw_payload(lw.load(core::sync::atomic::Ordering::Acquire)));
    let m = get_monitors().get(h);
    monitor_unlock(&m, me);
}

// === Public Lock API ===

pub fn obj_lock(obj: ObjectReference) {
    let lw = lockword(obj);
    let me = thread_id_u32();

    let mut cur = lw.load(core::sync::atomic::Ordering::Acquire);
    loop {
        match lw_state(cur) {
            LState::Unlocked => {
                let next = lw_compose(lw_ver(cur), thin_payload(me, 1), LState::Thin);
                if lw.compare_exchange_weak(cur, next,
                    core::sync::atomic::Ordering::AcqRel,
                    core::sync::atomic::Ordering::Acquire).is_ok() { 
                    return; 
                }
                cur = lw.load(core::sync::atomic::Ordering::Acquire);
            }
            LState::Thin => {
                let p = lw_payload(cur);
                if thin_owner(p) == me {
                    // reentrant
                    let rec = thin_rec(p).wrapping_add(1);
                    let next = lw_compose(lw_ver(cur), thin_payload(me, rec), LState::Thin);
                    if lw.compare_exchange_weak(cur, next, 
                        core::sync::atomic::Ordering::AcqRel,
                        core::sync::atomic::Ordering::Acquire).is_ok() { 
                        return; 
                    }
                    cur = lw.load(core::sync::atomic::Ordering::Acquire);
                } else {
                    // contention → inflate
                    inflate_and_lock(obj, lw, me);
                    return;
                }
            }
            LState::Inflated => { 
                inflated_lock(obj, lw, me); 
                return; 
            }
            _ => { 
                inflate_and_lock(obj, lw, me); 
                return; 
            }
        }
    }
}

pub fn obj_unlock(obj: ObjectReference) {
    let lw = lockword(obj);
    let me = thread_id_u32();

    let mut cur = lw.load(core::sync::atomic::Ordering::Acquire);
    loop {
        match lw_state(cur) {
            LState::Thin => {
                let p = lw_payload(cur);
                debug_assert_eq!(thin_owner(p), me);
                let rec = thin_rec(p);
                if rec > 1 {
                    let next = lw_compose(lw_ver(cur), thin_payload(me, rec - 1), LState::Thin);
                    if lw.compare_exchange_weak(cur, next,
                        core::sync::atomic::Ordering::AcqRel,
                        core::sync::atomic::Ordering::Acquire).is_ok() { 
                        return; 
                    }
                    cur = lw.load(core::sync::atomic::Ordering::Acquire);
                } else {
                    let next = lw_compose(lw_ver(cur).wrapping_add(1), 0, LState::Unlocked);
                    if lw.compare_exchange_weak(cur, next,
                        core::sync::atomic::Ordering::AcqRel,
                        core::sync::atomic::Ordering::Acquire).is_ok() { 
                        return; 
                    }
                    cur = lw.load(core::sync::atomic::Ordering::Acquire);
                }
            }
            LState::Inflated => { 
                inflated_unlock(obj, lw, me); 
                return; 
            }
            LState::Unlocked => panic!("unlock of unlocked object"),
            _ => panic!("unlock of non-locked state"),
        }
    }
}

// === Utility Functions ===

pub fn is_object_shared(obj: ObjectReference) -> bool {
    let lw = lockword(obj);
    let current = lw.load(Ordering::Acquire);
    !matches!(lw_state(current), LState::Unlocked)
}

pub struct ObjectLockGuard {
    obj: ObjectReference,
    was_already_locked: bool,
}

impl ObjectLockGuard {
    pub fn new(obj: ObjectReference) -> Self {
        let was_shared = is_object_shared(obj);
        if was_shared {
            obj_lock(obj);
        }
        Self {
            obj,
            was_already_locked: was_shared,
        }
    }
}

impl Drop for ObjectLockGuard {
    fn drop(&mut self) {
        if self.was_already_locked {
            obj_unlock(self.obj);
        }
    }
}