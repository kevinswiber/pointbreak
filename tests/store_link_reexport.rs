use pointbreak::session::{
    StoreLinkOptions, StoreLinkPreview, StoreLinkResult, StoreUnlinkOptions, StoreUnlinkResult,
    link_store_to_family, preview_link_to_family, unlink_store_from_family,
};

#[test]
fn store_link_seam_is_reachable_from_the_session_namespace() {
    // Compile-visibility only: naming each re-exported item (options via `::new`,
    // the fns as values, the results in a type position) proves the seam resolves
    // and keeps every import used under `-D warnings`.
    let _link_options = StoreLinkOptions::new(".", Some("fam".to_owned()));
    let _unlink_options = StoreUnlinkOptions::new(".");
    let _link_fn = link_store_to_family;
    let _preview_fn = preview_link_to_family;
    let _unlink_fn = unlink_store_from_family;
    let _link_result: Option<StoreLinkResult> = None;
    let _preview_result: Option<StoreLinkPreview> = None;
    let _unlink_result: Option<StoreUnlinkResult> = None;
}
