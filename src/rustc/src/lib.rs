#![feature(rustc_private)]
#![feature(const_collections_with_hasher)]
#![feature(const_trait_impl)]
#![feature(effects)]

extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hash;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_metadata;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

pub mod analyzer;
pub mod entry;
pub mod feeder;
pub mod hash;
