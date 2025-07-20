use mmtk::{util::{Address, ObjectReference}, vm::slot::{MemorySlice, Slot}};

use crate::value::{GcPtr, ValueRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlinkSlot {
    ObjectRef(Address),   // Points to raw ObjectReference
    OptionObjectRef(Address), // Points to Option<ObjectReference>
    ValueRef(Address),    // Points to ValueRef enum
}

impl Slot for BlinkSlot {
    fn load(&self) -> Option<ObjectReference> {
        println!("BlinkSlot::load called for {:?}", self);
        match self {
            BlinkSlot::ObjectRef(addr) => {
                let obj_ref = unsafe { (*addr).load::<ObjectReference>() };
                Some(obj_ref)
            },
            BlinkSlot::OptionObjectRef(addr) => {  // ← NEW!
                let opt_ref = unsafe { (*addr).load::<Option<ObjectReference>>() };
                opt_ref  // Returns Option<ObjectReference> directly
            },
            BlinkSlot::ValueRef(addr) => {
                let value_ref = unsafe { (*addr).load::<ValueRef>() };
                match value_ref {
                    ValueRef::Heap(gc_ptr) => Some(gc_ptr.0),
                    _ => None,
                }
            }
        }
    }
    
    fn store(&self, object: ObjectReference) {
        println!("BlinkSlot::store called for {:?}", self);
        unsafe {
            match self {
                BlinkSlot::ObjectRef(addr) => {
                    (*addr).store(object);
                },
                BlinkSlot::OptionObjectRef(addr) => {  // ← NEW!
                    (*addr).store(Some(object));
                },
                BlinkSlot::ValueRef(addr) => {
                    let value_ref = ValueRef::Heap(GcPtr::new(object));
                    (*addr).store(value_ref);
                }
            }
        }
    }
    
    fn prefetch_load(&self) { /* no-op */ }
    fn prefetch_store(&self) { /* no-op */ }
}



#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlinkMemorySlice {
    start: Address,
    bytes: usize,
}

pub struct BlinkSlotIterator {
    current: Address,
    end: Address,
    slot_size: usize,
}

impl Iterator for BlinkSlotIterator {
    type Item = BlinkSlot;
    
    fn next(&mut self) -> Option<Self::Item> {
        None
    }
}

impl MemorySlice for BlinkMemorySlice {
    type SlotType = BlinkSlot;
    
    fn start(&self) -> Address {
        self.start
    }
    
    fn bytes(&self) -> usize {
        self.bytes
    }
    
    fn copy(src: &Self, tgt: &Self) {
        unsafe {
            src.start.to_ptr::<u8>().copy_to_nonoverlapping(
                tgt.start.to_mut_ptr::<u8>(),
                src.bytes.min(tgt.bytes),
            );
        }
    }
    
    fn object(&self) -> Option<ObjectReference> {
        ObjectReference::from_raw_address(self.start)
    }
    
    type SlotIterator = BlinkSlotIterator;
    
    fn iter_slots(&self) -> Self::SlotIterator {
        let end = self.start + self.bytes;
        BlinkSlotIterator {
            current: self.start,
            end,
            slot_size: std::mem::size_of::<ObjectReference>(), // Fixed this line
        }
    }
}
