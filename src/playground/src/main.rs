use core::marker::PhantomData;

struct Token<T: ?Sized>(PhantomData<fn(T) -> T>);

struct A;
struct B;

fn main() {
    let mut a_src = Token::<A>(PhantomData);
    let mut b_src = Token::<B>(PhantomData);

    let a = dummy(&mut a_src);
    let b = dummy(&mut b_src);
    let _ = a;
}

fn dummy<'a, T: ?Sized>(_token: &'a mut Token<T>) -> &'a () {
    &()
}
