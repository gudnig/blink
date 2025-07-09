use mmtk::{util::{Address, ObjectReference}, vm::slot::{MemorySlice, Slot}};

// Slot and MemorySlice - basic memory operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlinkSlot(Address);

impl Slot for BlinkSlot {
    fn load(&self) -> Option<ObjectReference> {
        unsafe { Some(self.0.load::<ObjectReference>()) }  // Need to wrap in Some()
    }
    
    fn store(&self, object: ObjectReference) {
        unsafe { self.0.store(object) }
    }
    
    fn prefetch_load(&self) {
        // no-op by default
    }
    
    fn prefetch_store(&self) {
        // no-op by default
    }
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
        if self.current < self.end {
            let slot = BlinkSlot(self.current);
            self.current = self.current + self.slot_size;
            Some(slot)
        } else {
            None
        }
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
