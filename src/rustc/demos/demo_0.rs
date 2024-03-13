fn main() {}

fn whee<'a>() {
    // CanonicalUserTypeAnnotation {
    //     user_ty: Canonical {
    //         value: Ty(
    //             &ReLateParam(DefId(0:4 ~ demo_0[4d92]::whee), BrNamed(DefId(0:5 ~ demo_0[4d92]::whee::'a), 'a)) (),
    //         ),
    //         max_universe: U0,
    //         variables: [],
    //     },
    //     span: demo_0.rs:4:12: 4:18 (#0),
    //     inferred_ty: &ReErased (),
    // },
    let a: &'a () = unsafe { &*(0x1 as *mut ()) };
    __autoken_declare_tied_ref::<0, u32>();
}

fn __autoken_declare_tied_ref<const LT_ID: u32, T: ?Sized>() {}
