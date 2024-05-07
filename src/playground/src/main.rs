fn main() {}

struct Hehe;

fn whee<'a>() -> &'a Hehe {
    &Hehe
}
