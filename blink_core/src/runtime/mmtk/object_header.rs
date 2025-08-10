use mmtk::{util::{metadata::side_metadata::{SideMetadataOffset, SideMetadataSpec}, ObjectReference}, vm::ObjectModel};

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

#[derive(Copy, Clone, Eq, PartialEq)]
#[repr(u16)]
enum LState { Unlocked=0, Thin=1, Inflated=2, Hashed=3 }

#[inline] fn lw_state(x:u64)->LState { unsafe { core::mem::transmute((x & 0xFFFF) as u16) } }
#[inline] fn lw_ver(x:u64)->u64 { (x >> 56) & 0x7F }
#[inline] fn lw_payload(x:u64)->u64 { (x >> 16) & 0xFFFF_FFFFFF }
#[inline] fn lw_compose(ver:u64, payload:u64, st:LState)->u64 {
    ((ver & 0x7F) << 56) | ((payload & 0xFFFF_FFFFFF) << 16) | (st as u64)
}
#[inline] fn lw_bump_ver(x:u64)->u64 { lw_compose(lw_ver(x).wrapping_add(1), lw_payload(x), lw_state(x)) }

// Thin payload: [ rec:8 | owner_id:32 ] packed into 40 bits
#[inline] fn thin_payload(owner_id:u32, rec:u8)->u64 {
    ((owner_id as u64) & 0xFFFF_FFFF) | ((rec as u64) << 32)
}
#[inline] fn thin_owner(p:u64)->u32 { (p & 0xFFFF_FFFF) as u32 }
#[inline] fn thin_rec(p:u64)->u8 { (p >> 32) as u8 }

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

pub const OBJ_ID_META: SideMetadataSpec = SideMetadataSpec {
    name: "blink.obj_id",
    is_global: false,
    offset: SideMetadataOffset::,// No new method(0),                      // let MMTk place it
    
    log_num_of_bits: 6,             // 64 bits per object
    log_bytes_in_region: 0,            
};

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ObjId(pub u64);

fn obj_id(obj: ObjectReference) -> ObjId {
    
}



#[derive(Copy, Clone)]
struct MonHandle(u64); // <= 40 bits to fit payload

struct Monitor { /* owner, rec, queue, etc. */ }

struct Monitors { /* slab/arena */ }
static MONITORS: Monitors = Monitors::new();

impl Monitors {
    fn get_or_create(id: ObjId) -> MonHandle { /* … */ }
    fn get(h: MonHandle) -> &'static Monitor { /* … */ }
}

fn thread_id_u32() -> u32 { /* your goroutine/thread id */ }

fn inflate_and_lock(obj: ObjectReference,
    lw: &core::sync::atomic::AtomicU64,
    me: u32) {
let id = obj_id(obj);
let h = MONITORS.get_or_create(id); // off-heap

// Try to flag header as INFLATED(handle)
loop {
let cur = lw.load(core::sync::atomic::Ordering::Acquire);
if lw_state(cur) == LState::Inflated { break; }
let next = lw_compose(lw_ver(cur).wrapping_add(1), h.0 as u64, LState::Inflated);
if lw.compare_exchange_weak(cur, next,
core::sync::atomic::Ordering::AcqRel,
core::sync::atomic::Ordering::Acquire).is_ok() { break; }
}
inflated_lock(obj, lw, me)
}

fn inflated_lock(_obj: ObjectReference,
 lw: &core::sync::atomic::AtomicU64,
 me: u32) {
let h = MonHandle(lw_payload(lw.load(core::sync::atomic::Ordering::Acquire)));
let m = MONITORS.get(h);
monitor_lock(m, me); // park/unpark inside
}

fn inflated_unlock(_obj: ObjectReference,
   lw: &core::sync::atomic::AtomicU64,
   me: u32) {
let h = MonHandle(lw_payload(lw.load(core::sync::atomic::Ordering::Acquire)));
let m = MONITORS.get(h);
monitor_unlock(m, me);
}


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
                    core::sync::atomic::Ordering::Acquire).is_ok() { return; }
                cur = lw.load(core::sync::atomic::Ordering::Acquire);
            }
            LState::Thin => {
                let p = lw_payload(cur);
                if thin_owner(p) == me {
                    // reentrant
                    let rec = thin_rec(p).wrapping_add(1);
                    let next = lw_compose(lw_ver(cur), thin_payload(me, rec), LState::Thin);
                    if lw.compare_exchange_weak(cur, next, core::sync::atomic::Ordering::AcqRel,
                        core::sync::atomic::Ordering::Acquire).is_ok() { return; }
                    cur = lw.load(core::sync::atomic::Ordering::Acquire);
                } else {
                    // contention → inflate
                    inflate_and_lock(obj, lw, me);
                    return;
                }
            }
            LState::Inflated => { inflated_lock(obj, lw, me); return; }
            _ => { inflate_and_lock(obj, lw, me); return; } // e.g., HASHED
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
                        core::sync::atomic::Ordering::Acquire).is_ok() { return; }
                    cur = lw.load(core::sync::atomic::Ordering::Acquire);
                } else {
                    let next = lw_compose(lw_ver(cur).wrapping_add(1), 0, LState::Unlocked);
                    if lw.compare_exchange_weak(cur, next,
                        core::sync::atomic::Ordering::AcqRel,
                        core::sync::atomic::Ordering::Acquire).is_ok() { return; }
                    cur = lw.load(core::sync::atomic::Ordering::Acquire);
                }
            }
            LState::Inflated => { inflated_unlock(obj, lw, me); return; }
            LState::Unlocked => panic!("unlock of unlocked object"),
            _ => panic!("unlock of non-locked state"),
        }
    }
}
