// Welcome to the interactive Pre-RFC for `auto_context_tokens`!
//
// To type-check the project, run `make` in the default terminal directory. To run it, type `make run`.

#![allow(dead_code)]
#![allow(unused_imports)]
#![feature(arbitrary_self_types)]
#![feature(decl_macro)]
#![feature(lazy_cell)]

// The `auto_context_tokens` feature provides what is effectively syntactic sugar for passing "token"
// objects. This pattern is used by crates like `qcell` to provide a safe zero-cost `RefCell`
// mechanism that can be statically checked by the borrow checker.
mod old_token_mechanism {
    use qcell::{TCell, TCellOwner};

    struct MyToken;

    static MY_VEC: TCell<MyToken, Vec<u32>> = TCell::new(Vec::new());

    fn push_value(token: &mut TCellOwner<MyToken>, value: u32) {
        MY_VEC.rw(token).push(value);
    }

    fn iter_values(token: &TCellOwner<MyToken>) -> std::slice::Iter<'_, u32> {
        MY_VEC.ro(token).iter()
    }

    fn main() {
        let mut token = TCellOwner::new();
        push_value(&mut token, 4);
        push_value(&mut token, 5);

        for value in iter_values(&token) {
            dbg!(value);
        }

        // The following code will not borrow-check:
        // let ref_to_first = iter_values(&token).next().unwrap();
        // push_value(&mut token, 6);
        // let _ = ref_to_first;
    }
}

// Here's what that same code would look like with `auto_context_tokens`.
mod new_token_mechanism {
    use autoken::TokenCell;

    struct MyToken;

    static MY_VEC: TokenCell<Vec<u32>, MyToken> = TokenCell::new(Vec::new());

    fn push_value(value: u32) {
        // `get_mut()` magically borrows the token from scope.
        MY_VEC.get_mut().push(value);
    }

    fn iter_values<'a>() -> std::slice::Iter<'a, u32> {
        autoken::tie!('a => ref MyToken);

        MY_VEC.get().iter()
    }

    fn main() {
        push_value(4);
        push_value(5);

        for value in iter_values() {
            dbg!(value);
        }

        // The following code will not borrow-check: (uncomment it to see for yourself!)
        // let ref_to_first = iter_values().next().unwrap();
        // push_value(6);
        // let _ = ref_to_first;
    }
}

// Now, that may not seem like a big deal but this automated token passing mechanism can be used to
// automate all sorts of context-passing-heavy patterns. One particular pattern that comes to mind is
// Catherine West's arena pattern (https://www.youtube.com/watch?v=aKLntZcp27M).
mod tokened_arenas {
    use std::{
        marker::PhantomData,
        ops::{Deref, DerefMut},
        sync::LazyLock,
    };

    use autoken::TokenCell;
    use generational_arena::{Arena, Index};

    pub trait HasArena: Sized + 'static {
        fn arena_raw() -> &'static TokenCell<Arena<Self>>;
    }

    pub macro define_arena($($ty:ty),+$(,)?) {$(
        impl HasArena for $ty {
            fn arena_raw() -> &'static TokenCell<Arena<Self>> {
                static ARENA: LazyLock<TokenCell<Arena<$ty>>> =
                    LazyLock::new(|| TokenCell::new(Arena::new()));
                &*ARENA
            }
        }
    )*}

    pub struct Obj<T> {
        _ty: PhantomData<fn(T) -> T>,
        index: Index,
    }

    impl<T: HasArena> Copy for Obj<T> {}

    impl<T: HasArena> Clone for Obj<T> {
        fn clone(&self) -> Self {
            *self
        }
    }

    impl<T: HasArena> Obj<T> {
        pub fn new(value: T) -> Self {
            Self {
                _ty: PhantomData,
                index: T::arena_raw().get_mut().insert(value),
            }
        }

        pub fn get<'a>(&self) -> &'a T {
            autoken::tie!('a => ref Arena<T>);
            &T::arena_raw().get()[self.index]
        }

        pub fn get_mut<'a>(&self) -> &'a mut T {
            autoken::tie!('a => mut Arena<T>);
            &mut T::arena_raw().get_mut()[self.index]
        }

        pub fn destroy(self) {
            T::arena_raw().get_mut().remove(self.index);
        }
    }

    // This is a cute little hack.
    impl<T: HasArena> Deref for Obj<T> {
        type Target = T;

        fn deref<'a>(&'a self) -> &'a Self::Target {
            autoken::tie!('a => ref Arena<T>);
            self.get()
        }
    }

    impl<T: HasArena> DerefMut for Obj<T> {
        fn deref_mut<'a>(&'a mut self) -> &'a mut Self::Target {
            autoken::tie!('a => mut Arena<T>);
            self.get_mut()
        }
    }
}

// Look: a linked list!
mod arenas_demo {
    use crate::tokened_arenas::{define_arena, HasArena, Obj};

    define_arena!(LinkedList<i32>);

    fn main() {
        let a = Obj::new(LinkedList::new(1));
        let b = Obj::new(LinkedList::new(2));
        let c = Obj::new(LinkedList::new(3));

        a.insert_right(b);
        b.insert_right(c);
        a.iter_right(|val| {
            *val += 1;
            dbg!(*val);

            // Uncommenting this line will cause the program to be rejected.
            // c.destroy();
        });
    }

    pub struct LinkedList<T: 'static> {
        prev: Option<Obj<Self>>,
        next: Option<Obj<Self>>,
        value: T,
    }

    impl<T: 'static> LinkedList<T>
    where
        Self: HasArena,
    {
        pub fn new(value: T) -> Self {
            Self {
                prev: None,
                next: None,
                value,
            }
        }

        pub fn insert_right(mut self: Obj<Self>, mut node: Obj<Self>) {
            if let Some(mut next) = node.next {
                next.prev = Some(node);
            }
            node.next = self.next;
            node.prev = Some(self);
            self.next = Some(node);
        }

        pub fn iter_right(mut self: Obj<Self>, mut f: impl FnMut(&mut T)) {
            f(&mut self.value);

            while let Some(next) = self.next {
                self = next;
                f(&mut self.value);
            }
        }

        pub fn get_ith(mut self: Obj<Self>, index: usize) -> Obj<Self> {
            let mut counter = 0;

            if counter == 0 {
                return self;
            }

            while let Some(next) = self.next {
                self = next;
                counter += 1;

                if counter == index {
                    return self;
                }
            }

            panic!("failed to find element at index {index}");
        }
    }
}

// `auto_context_tokens` handles dynamic dispatch in a somewhat interesting way.
mod dynamic_dispatch {
    use core::fmt;
    use std::cell::RefCell;

    use autoken::{BorrowsAllExcept, TokenCell};

    static LIST: TokenCell<Vec<u32>> = TokenCell::new(Vec::new());

    // It would be really bad if this compiled but it doesn't.
    //     fn does_not_work() {
    //         let f: fn() = || {
    //             LIST.get_mut().push(3);
    //         };
    //
    //         LIST.get_mut().push(3);
    //         let value = &LIST.get()[0];
    //         // How are we supposed to know what this borrows?
    //         f();
    //         assert_eq!(*value, 3);
    //     }

    // This is because we reject all unsizing attempts that involve a function that borrows a token.

    // But what if we still wanted to borrow stuff in a dynamically dispatched function? The answer
    // is `BorrowsAllExcept`! This is a type which acquires every single token besides the set specified
    // by its generic parameter. Users can then call the `absorb()` method on the object and everything
    // borrowed by the closure the method receives will be forgotten by the token checker, allowing
    // the dynamic function to borrow-check.
    fn does_work() {
        let f: fn(&mut BorrowsAllExcept<'_, ()>) = |borrows| {
            borrows.absorb(|| {
                LIST.get_mut().push(3);
            });
        };

        LIST.get_mut().push(3);
        let value = &LIST.get()[0];
        f(&mut BorrowsAllExcept::acquire());

        // If we were to uncomment this line, the borrow checker would reject our code.
        // assert_eq!(*value, 3);
    }

    // This mechanism can be used to interoperate with code employing dynamic dispatch which didn't
    // explicitly add support for the `auto_context_tokens` feature.
    struct ListPrinter<'a>(RefCell<BorrowsAllExcept<'a, ()>>);

    impl<'a> ListPrinter<'a> {
        pub fn new() -> Self {
            autoken::tie!('a => except ());
            Self(RefCell::new(BorrowsAllExcept::acquire()))
        }
    }

    impl fmt::Debug for ListPrinter<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.borrow_mut().absorb(|| LIST.get().fmt(f))
        }
    }

    fn interop_demo() {
        dbg!(ListPrinter::new());
    }

    fn thread_demo() {
        let mut borrows = BorrowsAllExcept::<()>::acquire();

        std::thread::scope(|s| {
            s.spawn(|| {
                borrows.absorb(|| {
                    LIST.get_mut().push(3);
                });
            });
        });

        LIST.get_mut().pop().unwrap();
    }
}

// Finally, let's look under the covers to see the two major primitives that power this entire feature.
mod feature_internals {
    fn tie_one<'a>() -> &'a () {
        // The first is `tie`. This directive lets you declare lifetime parameters in the return type
        // of the current function which are "tied" to a global token.
        autoken::tie!('a => ref u32);
        autoken::tie!('a => mut i32);

        &()
    }

    // The `tie` directive also lets you declare that the function acquires a token for its duration
    // without tying a return lifetime to that borrow.
    fn just_borrow_temp() {
        autoken::tie!(ref i32);
    }

    // Finally, you can tie everything with the exception of whatever is in the provided list with
    // the except modifier. This is how we implement `BorrowsAllExcept::acquire`.
    fn tie_all<'a>() -> &'a () {
        autoken::tie!('a => except (u32, i32, f32));
        &()
    }

    // To implement the `absorb` method, we just use the more primitive `unsafe` method called
    // `absorb_borrows_except`.
    fn absorb_demo() {
        let _func: fn() = || unsafe {
            autoken::absorb_borrows_except::<(u32, i32), _>(|| {
                autoken::tie!(ref f32);
                autoken::tie!(mut f64);
                autoken::tie!(except(u32, i32));

                // Uncommenting any of these would cause an error.
                // autoken::tie!(ref u32);
                // autoken::tie!(mut i32);
                // autoken::tie!(except ());
            });
        };
    }
}

// Now it's your turn to play around with `auto_context_tokens`. There are some caveats with the
// prototype however...
//
// BUGS:
// 1. Recursive functions will probably ICE or cause incorrect results because I suck at algorithms
//    on graphs. This is purely a skill issue and I'm going to fix it soon.
//
// 2. Tied lifetime identification is kinda sketchy. Don't be surprised if it ICEs or otherwise
//    produces an unsound result—they're really hard to get right! (please do report these incidents,
//    though)
//
// 3. Calling generic functions with borrows across crate boundaries is almost certainly unsound because
//    of a major limitation in my analyzer design: we can only analyze crate-local functions whose
//    generic parameters are known. In other words, these functions just aren't analyzed...
//
// QOL:
//
// 1. Incremental compilation doesn't work so you just have to clean the entire project and recompile.
//    This is because I'm abusing the query system to have it do things it was never meant to do.
//
// 2. The diagnostics are terrible. Have fun tracking down your mistakes!
//
// 3. The tool is generally very slow because it has to generate a shadow MIR body for every concrete
//    instantiation of a function it can find to borrow-check token accesses. Also, there's no caches
//    so a bunch of analyses for functions need to be repeated. I'm pretty sure this tool is O(n^2)
//    w.r.t crate depth. This slowness is one of the biggest open questions for this proposal—see below!
//
// OPEN QUESTIONS:
//
// 1. As previously mentioned, we have to analyze each and every generic instantiation reachable in the
//    crate tree separately. The query system (and the borrow checker in general) really don't like this
//    so we're definitely going to need a better solution. There's two major ways I see of fixing this:
//
//    1) Use information from the borrow checker to gather a set of generic parameters which can't alias
//    as well as a set of tokens a generically-indeterminate function target can't borrow safely. I
//    have no clue whether the borrow checker can do stuff like this.
//
//    2) Have users annotate this information manually. I don't really know how to design this solution
//       and make it convenient.
//
// 2. Although `BorrowsAllExcept::absorb` successfully makes dynamic dispatch convenient and multithreaded
//    borrows functional, it doesn't work when you combine these two things. That is, if you use
//   `BorrowsAllExcept::acquire`, you're locking yourself into a single-threaded program for the
//   duration of that borrow. There's two major solutions I can think of to this problem:
//
//   1) Make tokens thread-local by resetting one's borrow set across thread boundaries. This makes
//      dynamic dispatch convenient again but forces users to use thread-local storage to make their
//      globals work again. This is bad for `no_std` environments and, iirc, TLS is considerably slower
//      than a regular global.
//
//   2) Create a `BorrowsOnly` token and provide set operations like unions and, importantly, removals,
//      to emulate `BorrowsAllExcept`'s behavior. This solution is probably convenient but I'm very
//      worried about its computational complexity since people are probably going to stuff all sorts
//      of tokens into these sets.
//
// Anyways, have fun with the demo!

use autoken::{tie, BorrowsAllExcept, TokenCell};
use tokened_arenas::{define_arena, HasArena, Obj};

fn main() {
    println!("Hello, World!");
}
