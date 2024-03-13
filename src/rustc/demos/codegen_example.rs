// See https://doc.rust-lang.org/stable/nightly-rustc/rustc_middle/ty/typeck_results/type.CanonicalUserTypeAnnotations.html
// for an idea of how to implement this in the MIR builder.
mod demo {
    use std::cell::{Ref, RefCell};

    type Binder<'a> = (&'a mut (), Ref<'a, u32>);

    fn fake_it<T>() -> T {
        unreachable!();
    }

    // CanonicalUserTypeAnnotation {
    //     user_ty: Canonical {
    //         value: Ty(
    //             (&ReBound(DebruijnIndex(0), BoundRegion { var: 0, kind: BrAnon }) mut (), std::cell::Ref<ReBound(DebruijnIndex(0), BoundRegion { var: 0, kind: BrAnon }), u32>),
    //         ),
    //         max_universe: U0,
    //         variables: [
    //             CanonicalVarInfo {
    //                 kind: Region(
    //                     U0,
    //                 ),
    //             },
    //         ],
    //     },
    //     span: demo_0.rs:18:14: 18:24 (#0),
    //     inferred_ty: (&ReErased mut (), std::cell::Ref<ReErased, u32>),
    // },
    fn foo3<'a, 'b>() -> Ref<'a, u32> {
        let mut token = ();
        let token: &mut () = &mut token;
        let cell: &'static RefCell<u32> = fake_it();

        // ...

        let bar: Binder<'_> = (token, cell.borrow());
        bar.1
    }

    fn foo4<'a, 'b>() -> Ref<'a, u32> {
        let token: &'a mut () = fake_it();
        let cell: &'static RefCell<u32> = fake_it();

        // ...

        let bar: Binder<'_> = (token, cell.borrow());
        bar.1
    }

    fn foo5<'a, 'b>() -> Ref<'b, u32> {
        let token: &'a mut () = fake_it();
        let cell: &'static RefCell<u32> = fake_it();

        // ...

        let bar: Binder<'_> = (token, cell.borrow());
        bar.1
    }
}

mod demo2 {
    // Untied header
    fn foo1() {
        let local = ();
        let token: &'_ () = &local;

        // ...
    }

    // Tied header
    fn foo2<'a>() {
        let token: &'a () = &(); // (dangling)

        // ...
    }
}
