macro_rules! foo {
    () => { let x = 1; }
}

fn main() {
    foo!();
}