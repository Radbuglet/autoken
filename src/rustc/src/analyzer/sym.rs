#![allow(non_upper_case_globals)]

use crate::util::mir::CachedSymbol;

pub static __autoken_declare_tied: CachedSymbol = CachedSymbol::new("__autoken_declare_tied");

pub static __autoken_absorb_only: CachedSymbol = CachedSymbol::new("__autoken_absorb_only");

pub static __autoken_mut_ty_marker: CachedSymbol = CachedSymbol::new("__autoken_mut_ty_marker");

pub static __autoken_ref_ty_marker: CachedSymbol = CachedSymbol::new("__autoken_ref_ty_marker");

pub static __autoken_downgrade_ty_marker: CachedSymbol =
    CachedSymbol::new("__autoken_downgrade_ty_marker");

pub static __autoken_diff_ty_marker: CachedSymbol = CachedSymbol::new("__autoken_diff_ty_marker");

pub static unnamed: CachedSymbol = CachedSymbol::new("unnamed");
