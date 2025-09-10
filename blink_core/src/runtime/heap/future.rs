use std::mem;

use mmtk::util::ObjectReference;
use mmtk::Mutator;

use crate::future::BlinkFuture;
use crate::runtime::BlinkVM;

impl BlinkVM {
    pub fn alloc_future(&self, future: BlinkFuture) -> ObjectReference {
        self.with_mutator(|mutator| {
            // BlinkFuture is a complex type, but we only need to store a reference to it
            // The actual future data will be managed separately in Rust heap
            // We'll allocate minimal space and store a pointer/handle
            
            let size = mem::size_of::<*const BlinkFuture>();
            let obj_ref = mutator
                .alloc(size, mem::align_of::<*const BlinkFuture>(), 0, mmtk::AllocationSemantics::Default)
                .expect("Failed to allocate future object");

            // Store the future by boxing it and storing the raw pointer
            let boxed_future = Box::new(future);
            let future_ptr = Box::into_raw(boxed_future);
            
            unsafe {
                let data_ptr = obj_ref.to_raw_address().as_usize() as *mut *const BlinkFuture;
                std::ptr::write(data_ptr, future_ptr);
            }

            obj_ref
        })
    }
}