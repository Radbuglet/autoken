fn main() {
    loop {
        borrow_mutably::<Nothing>();
        borrow_mutably::<Nothing>();
        borrow_mutably::<Nothing>();
    }
}

pub struct Nothing {
    __autoken_nothing_type_field_indicator: (),
}

pub const fn borrow_mutably<T: ?Sized>() {
    const fn __autoken_borrow_mutably<T: ?Sized>() {}

    __autoken_borrow_mutably::<T>();
}
