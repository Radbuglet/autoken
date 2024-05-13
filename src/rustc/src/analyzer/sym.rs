#![allow(non_upper_case_globals)]

use crate::util::mir::CachedSymbol;

macro_rules! define {
    ($($ident:ident)*) => {
        $(pub static $ident: CachedSymbol = CachedSymbol::new(stringify!($ident));)*
    };
}

define! {
    __autoken_declare_tied
    __autoken_absorb_only
    __autoken_mut_ty_marker
    __autoken_ref_ty_marker
    __autoken_downgrade_ty_marker
    __autoken_diff_ty_marker
    unnamed
}

pub static ANON_LT: CachedSymbol = CachedSymbol::new("'_");
