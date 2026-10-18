//! Record store adapters beside Beads ([`crate::beads`]): the Virgil Records
//! adapter, which holds both the private and the shared store, and the
//! export that moves Beads records into Virgil.

pub mod export;
pub mod virgil;
