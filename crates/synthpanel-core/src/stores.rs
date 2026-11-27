//! Small store helpers grouped for the IPC dispatch layer.
//!
//! These functions work directly on the shared SQLite pool rather than on a
//! repository struct, so the dispatch code can call them without threading
//! another dependency through `Stores`. They are re-exported from the modules
//! that own the underlying tables; both paths are equivalent.

pub use crate::site_acl::{
    list_site_ids_for_user, set_sites_for_user, site_ids_contain, user_can_access_site,
};
pub use crate::sites::enforce_db_quota;
