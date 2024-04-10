use autoken::{Mut, Ref, TokenSet};

fn main() {
    incr_demo::<Ref<u32>>();
}

fn incr_demo<Normal: TokenSet>() {
    unsafe {
        autoken::absorb::<Mut<u32>, _>(|| {
            fn dummy_borrow_set<'a, T: TokenSet>() -> &'a () {
                autoken::tie!('a => set T);
                &()
            }

            let all_borrow = dummy_borrow_set::<Mut<u32>>();
            autoken::absorb::<Normal, _>(|| {
                autoken::tie!(mut u32);
            });
            let _ = all_borrow;
        });
    }
}
