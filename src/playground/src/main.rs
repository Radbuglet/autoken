struct A;
struct B;

fn main() {
    let mut a_src = A;
    let mut b_src = B;

    let a = &mut a_src;
    let b = &mut b_src;
    // let _ = a;
}
