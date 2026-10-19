//! Record store adapters beside Beads ([`crate::beads`]): the Virgil Records
//! adapter, which holds both the private and the shared store, the export
//! that moves Beads records into Virgil, and the export that writes them as
//! the rulebook file.

pub mod export;
pub mod rulebook;
pub mod virgil;
