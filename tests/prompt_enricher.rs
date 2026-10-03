use immich_analyze::prompt_enricher::PromptContext;

const BASE_PROMPT: &str = "Describe the image.";

#[test]
fn context_without_metadata_leaves_the_prompt_untouched() {
    let context = PromptContext::new(BASE_PROMPT);

    assert_eq!(context.build_enriched_prompt(), BASE_PROMPT);
}

#[test]
fn mime_types_become_readable_asset_types() {
    let jpeg = PromptContext::new(BASE_PROMPT)
        .with_file_info(Some("photo.jpg".to_owned()), Some("IMAGE".to_owned()))
        .with_mime_type(Some("image/jpeg".to_owned()))
        .build_enriched_prompt();
    let raw = PromptContext::new(BASE_PROMPT)
        .with_file_info(Some("photo.dng".to_owned()), Some("IMAGE".to_owned()))
        .with_mime_type(Some("image/dng".to_owned()))
        .build_enriched_prompt();
    let video = PromptContext::new(BASE_PROMPT)
        .with_file_info(Some("clip.mp4".to_owned()), Some("VIDEO".to_owned()))
        .with_mime_type(Some("video/mp4".to_owned()))
        .build_enriched_prompt();

    assert!(jpeg.contains("Asset type: JPEG photo"));
    assert!(raw.contains("Asset type: RAW photo (DNG)"));
    assert!(video.contains("Asset type: MP4 video"));
}

#[test]
fn unknown_mime_types_fall_back_to_the_raw_asset_type() {
    let prompt = PromptContext::new(BASE_PROMPT)
        .with_file_info(Some("clip.bin".to_owned()), Some("OTHER".to_owned()))
        .with_mime_type(Some("application/octet-stream".to_owned()))
        .build_enriched_prompt();

    assert!(prompt.contains("Asset type: OTHER"));
}

#[test]
fn asset_type_without_a_mime_type_is_passed_through() {
    let prompt = PromptContext::new(BASE_PROMPT)
        .with_file_info(None, Some("IMAGE".to_owned()))
        .build_enriched_prompt();

    assert!(prompt.contains("Asset type: IMAGE"));
}

#[test]
fn resolution_is_only_reported_when_both_dimensions_are_known() {
    let complete = PromptContext::new(BASE_PROMPT)
        .with_resolution(Some(4032_i32), Some(3024_i32))
        .build_enriched_prompt();
    let partial = PromptContext::new(BASE_PROMPT)
        .with_resolution(Some(4032_i32), None)
        .build_enriched_prompt();
    let negative = PromptContext::new(BASE_PROMPT)
        .with_resolution(Some(-1_i32), Some(3024_i32))
        .build_enriched_prompt();

    assert!(complete.contains("Resolution: 4032×3024"));
    assert!(!partial.contains("Resolution:"));
    assert!(!negative.contains("Resolution:"));
}

#[test]
fn people_are_rendered_with_their_age_at_photo_time() {
    let prompt = PromptContext::new(BASE_PROMPT)
        .with_people(vec![
            ("Ann".to_owned(), Some(34)),
            ("Baby".to_owned(), Some(0)),
            ("Guest".to_owned(), None),
        ])
        .build_enriched_prompt();

    assert!(prompt.contains("People: Ann (34 years), Baby (<1 year), Guest"));
}

#[test]
fn tags_are_listed_verbatim() {
    let prompt = PromptContext::new(BASE_PROMPT)
        .with_tags(vec!["family".to_owned(), "beach".to_owned()])
        .build_enriched_prompt();

    assert!(prompt.contains("Tags: family, beach"));
}

#[test]
fn camera_is_reported_with_and_without_a_model() {
    let both = PromptContext::new(BASE_PROMPT)
        .with_camera_info(Some("Apple".to_owned()), Some("iPhone 15 Pro".to_owned()))
        .build_enriched_prompt();
    let make_only = PromptContext::new(BASE_PROMPT)
        .with_camera_info(Some("Apple".to_owned()), None)
        .build_enriched_prompt();

    assert!(both.contains("Camera: Apple iPhone 15 Pro"));
    assert!(make_only.contains("Camera: Apple"));
}

#[test]
fn exposure_settings_are_formatted_per_value() {
    let full = PromptContext::new(BASE_PROMPT)
        .with_exposure_settings(
            Some("1/250".to_owned()),
            Some(2.8_f64),
            Some(35.4_f64),
            Some(400),
        )
        .build_enriched_prompt();
    let partial = PromptContext::new(BASE_PROMPT)
        .with_exposure_settings(None, None, None, Some(100))
        .build_enriched_prompt();

    assert!(full.contains("Exposure: 1/250s, f/2.8, 35mm, ISO 400"));
    assert!(partial.contains("Exposure: ISO 100"));
}

#[test]
fn an_empty_exif_description_is_not_repeated_back() {
    let prompt = PromptContext::new(BASE_PROMPT)
        .with_exif_description(Some(String::new()))
        .build_enriched_prompt();

    assert_eq!(prompt, BASE_PROMPT);
}

#[test]
fn the_full_context_is_assembled_in_a_stable_order() {
    let prompt = PromptContext::new(BASE_PROMPT)
        .with_file_info(Some("IMG_0001.HEIC".to_owned()), Some("IMAGE".to_owned()))
        .with_mime_type(Some("image/heic".to_owned()))
        .with_resolution(Some(4032_i32), Some(3024_i32))
        .with_created_at(Some("2024-05-01T10:00:00.000Z".to_owned()))
        .with_time_zone(Some("Europe/Lisbon".to_owned()))
        .with_location(Some("Lisbon, Lisboa, Portugal".to_owned()))
        .with_people(vec![("Ann".to_owned(), Some(34))])
        .with_tags(vec!["family".to_owned()])
        .with_camera_info(Some("Apple".to_owned()), Some("iPhone 15 Pro".to_owned()))
        .with_lens_model(Some("iPhone 15 Pro back camera".to_owned()))
        .with_exposure_settings(
            Some("1/250".to_owned()),
            Some(2.8_f64),
            Some(35.4_f64),
            Some(400),
        )
        .with_rating(Some(4))
        .with_exif_description(Some("Sunset on the Tagus".to_owned()))
        .build_enriched_prompt();

    assert_eq!(
        prompt,
        concat!(
            "Describe the image.\n\nAdditional context:\n",
            "Asset type: HEIC photo\n",
            "Resolution: 4032×3024\n",
            "Date taken: 2024-05-01T10:00:00.000Z\n",
            "Time zone: Europe/Lisbon\n",
            "Location: Lisbon, Lisboa, Portugal\n",
            "People: Ann (34 years)\n",
            "Tags: family\n",
            "Camera: Apple iPhone 15 Pro\n",
            "Lens: iPhone 15 Pro back camera\n",
            "Exposure: 1/250s, f/2.8, 35mm, ISO 400\n",
            "Rating: 4/5\n",
            "Original filename: IMG_0001.HEIC\n",
            "Existing description: Sunset on the Tagus",
        )
    );
}
