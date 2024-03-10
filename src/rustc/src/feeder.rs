#![allow(clippy::missing_safety_doc)]

use std::{any::TypeId, mem::transmute, ptr::NonNull, sync::RwLock};

use rustc_hir::def_id::DefId;
use rustc_middle::ty::TyCtxt;
use scopeguard::guard;
use smallbox::{space::S2, SmallBox};

use crate::hash::{ConstSafeBuildHasherDefault, FxHashMap};

// Store
static FEEDER: RwLock<FeederState> = RwLock::new(FeederState {
    tcx_addr: None,
    mappings: FxHashMap::with_hasher(ConstSafeBuildHasherDefault::new()),
});

struct FeederState {
    tcx_addr: Option<NonNull<()>>,
    mappings: FxHashMap<(TypeId, DefId), ErasedFeedValue<'static>>,
}

unsafe impl Send for FeederState {}
unsafe impl Sync for FeederState {}

type ErasedFeedValue<'tcx> = SmallBox<dyn ReallyAny + 'tcx, S2>;

trait ReallyAny {}

impl<T: ?Sized> ReallyAny for T {}

// Methods
fn tcx_addr(tcx: TyCtxt<'_>) -> NonNull<()> {
    NonNull::from(&**tcx).cast()
}

pub fn enter_feeder<R>(tcx: TyCtxt<'_>, f: impl FnOnce() -> R) -> R {
    let mut feeder = FEEDER.write().unwrap();
    assert_eq!(feeder.tcx_addr, None);
    assert!(feeder.mappings.is_empty());
    feeder.tcx_addr = Some(tcx_addr(tcx));
    drop(feeder);

    let gua = guard((), |()| FEEDER.write().unwrap().tcx_addr = None);
    let res = f();
    drop(gua);
    res
}

pub fn feed<'tcx, F: Feedable>(tcx: TyCtxt<'tcx>, id: DefId, val: F::Fed<'tcx>) {
    let mut feeder = FEEDER.write().unwrap();
    assert_eq!(feeder.tcx_addr, Some(tcx_addr(tcx)));
    feeder.mappings.insert((TypeId::of::<F>(), id), unsafe {
        let val: SmallBox<dyn ReallyAny + 'tcx, S2> = smallbox::smallbox!(val);
        transmute::<ErasedFeedValue<'tcx>, ErasedFeedValue<'static>>(val)
    });
}

pub fn read_feed<F: Feedable>(tcx: TyCtxt<'_>, id: DefId) -> Option<F::Fed<'_>> {
    let feeder = FEEDER.read().unwrap();
    assert_eq!(feeder.tcx_addr, Some(tcx_addr(tcx)));

    feeder
        .mappings
        .get(&(TypeId::of::<F>(), id))
        .map(|v| unsafe { *(&**v as &dyn ReallyAny as *const dyn ReallyAny as *const F::Fed<'_>) })
}

// Feedable
pub unsafe trait Feedable: 'static {
    type Fed<'tcx>: Send + Sync + Copy;
}

macro_rules! define_feedable {
    ($($name:ident => $ty:ty),*$(,)?) => {$(
        #[non_exhaustive]
        pub struct $name;

        unsafe impl $crate::feeder::Feedable for $name {
            type Fed<'tcx> = $ty;
        }
    )*};
}

pub(crate) use define_feedable;

pub mod feeders {
    use rustc_data_structures::steal::Steal;
    use rustc_hir::Constness;
    use rustc_middle::mir::Body;

    super::define_feedable! {
        MirBuiltFeeder => &'tcx Steal<Body<'tcx>>,
        ConstnessFeeder => Constness,
    }
}

// === `store_previous` macro === //

#[doc(hidden)]
pub mod once_val_macro_internals {
    use std::sync::OnceLock;

    pub struct MyOnceLock<T>(OnceLock<T>);

    impl<T: Copy> MyOnceLock<T> {
        pub const fn new() -> Self {
            Self(OnceLock::new())
        }

        pub fn init(&self, value: T) {
            self.0
                .set(value)
                .ok()
                .expect("override container initialized more than once")
        }

        pub fn get(&self) -> T {
            *self.0.get().expect("override container never initialized")
        }
    }
}

macro_rules! once_val {
    ($(
        $vis:vis $name:ident: $ty:ty = $expr:expr;
    )*) => {$(
        #[allow(non_upper_case_globals)]
        static $name: $crate::feeder::once_val_macro_internals::MyOnceLock<$ty> =
            $crate::feeder::once_val_macro_internals::MyOnceLock::new();

        $name.init($expr);
    )*};
}

pub(crate) use once_val;
