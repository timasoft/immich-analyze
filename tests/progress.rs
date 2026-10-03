use immich_analyze::progress::Indicator;

#[test]
fn the_indicator_starts_empty_and_tracks_the_finish_message() {
    let indicator = Indicator::new(3, "All done");

    assert_eq!(indicator.total, 3);
    assert_eq!(indicator.current, 0);
    assert!(indicator.current_message.is_empty());
    assert_eq!(indicator.finish_message, "All done");
}

#[test]
fn counting_assets_updates_current_and_total() {
    let mut indicator = Indicator::new(4, "All done");

    indicator.set_message("processing asset 1");
    assert_eq!(indicator.current, 0);
    assert_eq!(indicator.current_message, "processing asset 1");

    indicator.inc();
    assert_eq!(indicator.current, 1);

    indicator.set_message_and_inc("finished asset 2");
    assert_eq!(indicator.current, 2);
    assert_eq!(indicator.current_message, "finished asset 2");

    indicator.set_message_and_dec_total("skipped asset 3");
    assert_eq!(indicator.current, 2);
    assert_eq!(indicator.total, 3);
    assert_eq!(indicator.current_message, "skipped asset 3");

    indicator.dec_total();
    assert_eq!(indicator.total, 2);
}

#[test]
fn counters_saturate_instead_of_underflowing() {
    let mut indicator = Indicator::new(0, "All done");

    indicator.dec_total();

    assert_eq!(indicator.total, 0);
}
