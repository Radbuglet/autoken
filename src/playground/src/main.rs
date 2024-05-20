use autoken::Brand;
use std::{pin::pin, future::{Future, pending}};

fn main() {}

fn wazoo() {
    let _ = pin!(foo()).poll(&mut todo());
}

async fn foo() {
    let brand = Brand::<u32>::acquire_mut();
    pending::<()>().await;
    let _ = brand;
}

fn todo<T>() -> T {
    todo!();
}
