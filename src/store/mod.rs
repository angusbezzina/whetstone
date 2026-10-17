//! Record store adapters beside Beads ([`crate::beads`]): the private file
//! store the Virgil arm keeps drafts and receipts in, the Virgil Records
//! adapter, and the export that moves Beads records into Virgil.

pub mod export;
pub mod file;
pub mod virgil;
