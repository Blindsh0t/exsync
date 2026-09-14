#[test]
fn test_no_args() {
    let args = vec![];
    let res = super::parse_args(args.into_iter()).unwrap();
    assert!(!res.dry_run);
}

#[test]
fn test_dry_run() {
    let args = vec!["--dry-run".to_string()];
    let res = super::parse_args(args.into_iter()).unwrap();
    assert!(res.dry_run);
}

#[test]
fn test_bogus_args() {
    let args1 = vec!["--config".to_string(), "x".to_string()];
    assert_eq!(super::parse_args(args1.into_iter()), Err(2));

    let args2 = vec!["--bogus".to_string()];
    assert_eq!(super::parse_args(args2.into_iter()), Err(2));
}
