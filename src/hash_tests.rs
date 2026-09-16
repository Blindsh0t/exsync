use crate::hash::{fnv1a64, Fnv1a};

#[test]
fn ut08_empty() {
    assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
}

#[test]
fn ut08_a() {
    assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
}

#[test]
fn ut08_foobar_oneshot() {
    assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
}

#[test]
fn ut08_foobar_chunked() {
    let mut h = Fnv1a::new();
    h.write(b"foo");
    h.write(b"bar");
    assert_eq!(h.finish(), fnv1a64(b"foobar"));
}
