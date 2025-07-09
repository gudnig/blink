use mmtk::{util::ObjectReference, vm::{Scanning, VMBinding}, Mutator};

use crate::runtime::BlinkVM;


// Minimal Scanning implementation for NoGC
pub struct BlinkScanning;

impl Scanning<BlinkVM> for BlinkScanning {
    fn scan_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        _tls: mmtk::util::VMWorkerThread,
        _object: ObjectReference,
        _slot_visitor: &mut SV,
    ) {
        // For NoGC, we don't need to scan objects
        // This would be implemented for actual GC plans
    }

    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: mmtk::util::VMWorkerThread) {
        // No-op for NoGC
    }

    fn scan_roots_in_mutator_thread(
        _tls: mmtk::util::VMWorkerThread,
        _mutator: &'static mut Mutator<BlinkVM>,
        _factory: impl mmtk::vm::RootsWorkFactory<<BlinkVM as VMBinding>::VMSlot>,
    ) {
        // For NoGC, we don't scan roots
    }

    fn scan_vm_specific_roots(
        _tls: mmtk::util::VMWorkerThread,
        _factory: impl mmtk::vm::RootsWorkFactory<<BlinkVM as VMBinding>::VMSlot>
    ) {
        // For NoGC, we don't scan roots
    }

    fn supports_return_barrier() -> bool {
        false // No return barriers for NoGC
    }

    fn prepare_for_roots_re_scanning() {
        // No-op for NoGC
    }
}