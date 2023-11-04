trait Whee<T> {
    fn woo<'a>(&self, a: &'a u32, b: &'a u32) -> (&'a u32, T) {
        (a, loop {})
    }
}

impl Whee<u32> for bool {
    fn woo<'a>(&self, a: &'a u32, b: &'a u32) -> (&'a u32, u32) {
        if *self {
            (a, 5)
        } else {
            (b, 4)
        }
    }
}

impl Whee<u32> for u8 {
    fn woo<'a>(&self, a: &'a u32, b: &'a u32) -> (&'a u32, u32) {
        borrow_mutably::<u32>();

        if *self == 0 {
            (a, 5)
        } else {
            (b, 4)
        }
    }
}

fn main() {
    let bar = &4u8 as &dyn Whee<u32>;
    let foo = &true as &dyn Whee<u32>;
    foo.woo(&2, &3);

    borrow_mutably::<u32>();
}

fn borrow_mutably<T: ?Sized>() {
    fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}

fn borrow_immutably<T: ?Sized>() {
    fn __autoken_borrow_immutably<T: ?Sized>() {}

    __autoken_borrow_immutably::<T>();
}

fn unborrow_mutably<T: ?Sized>() {
    fn __autoken_unborrow_mutably<T: ?Sized>() {}

    __autoken_unborrow_mutably::<T>();
}

fn unborrow_immutably<T: ?Sized>() {
    fn __autoken_unborrow_immutably<T: ?Sized>() {}

    __autoken_unborrow_immutably::<T>();
}
