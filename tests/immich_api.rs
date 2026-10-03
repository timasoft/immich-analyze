//! Tests for [`immich_analyze::immich_api`]: provider configuration and the deserialization of the
//! Immich REST payloads. The HTTP round trips are deliberately not covered — they would require a
//! mock server.

use immich_analyze::{
    error::AnalysisError,
    immich_api::{ApiProvider, AssetMetadata, AssetResponse},
};

const FULL_METADATA: &str = r#"{
    "originalFileName": "IMG_0001.HEIC",
    "type": "IMAGE",
    "fileCreatedAt": "2024-05-01T10:00:00.000Z",
    "localDateTime": "2024-05-01T13:00:00.000Z",
    "width": 4032,
    "height": 3024,
    "originalMimeType": "image/heic",
    "people": [
        { "name": "Ann", "birthDate": "1990-01-15" },
        { "name": "Guest" }
    ],
    "tags": [{ "value": "family" }, { "value": "beach" }],
    "exifInfo": {
        "description": "Sunset on the Tagus",
        "city": "Lisbon",
        "state": "Lisboa",
        "country": "Portugal",
        "make": "Apple",
        "model": "iPhone 15 Pro",
        "dateTimeOriginal": "2024-04-30T18:22:00.000Z",
        "lensModel": "iPhone 15 Pro back camera",
        "exposureTime": "1/250",
        "fNumber": 2.8,
        "focalLength": 35.4,
        "iso": 400,
        "rating": 4,
        "timeZone": "Europe/Lisbon"
    }
}"#;

#[test]
fn provider_needs_a_parseable_url_and_at_least_one_key() {
    assert!(matches!(
        ApiProvider::new("not a url", &["key".to_owned()]),
        Err(AnalysisError::InvalidConfig { .. })
    ));
    assert!(matches!(
        ApiProvider::new("https://immich.example.com", &[]),
        Err(AnalysisError::InvalidConfig { .. })
    ));
    assert!(ApiProvider::new("https://immich.example.com", &["key".to_owned()]).is_ok());
    assert!(
        ApiProvider::new(
            "https://immich.example.com/",
            &["key-one".to_owned(), "key-two".to_owned()]
        )
        .is_ok()
    );
}

#[test]
fn api_keys_that_cannot_be_http_headers_are_rejected() {
    let with_newline = ApiProvider::new("https://immich.example.com", &["bad\nkey".to_owned()]);
    let with_space = ApiProvider::new("https://immich.example.com", &["bad key".to_owned()]);

    assert!(matches!(with_newline, Err(AnalysisError::InvalidApiKey)));
    assert!(with_space.is_ok(), "spaces are valid header values");
}

#[test]
fn the_debug_representation_never_leaks_api_keys() {
    let provider = ApiProvider::new(
        "https://immich.example.com",
        &["super-secret-key".to_owned()],
    )
    .expect("valid configuration");

    let rendered = format!("{provider:?}");

    assert!(!rendered.contains("super-secret-key"));
    assert!(rendered.contains("immich.example.com"));
    assert!(rendered.contains("1 clients"));
}

#[test]
fn asset_metadata_is_read_from_a_full_immich_payload() {
    let metadata: AssetMetadata =
        serde_json::from_str(FULL_METADATA).expect("camelCase payload deserializes");

    assert_eq!(
        metadata.original_file_name.as_deref(),
        Some("IMG_0001.HEIC")
    );
    assert_eq!(metadata.r#type.as_deref(), Some("IMAGE"));
    assert_eq!(
        metadata.file_created_at.as_deref(),
        Some("2024-05-01T10:00:00.000Z")
    );
    assert_eq!(
        metadata.local_date_time.as_deref(),
        Some("2024-05-01T13:00:00.000Z")
    );
    assert_eq!(metadata.width, Some(4032_i32));
    assert_eq!(metadata.height, Some(3024_i32));
    assert_eq!(metadata.original_mime_type.as_deref(), Some("image/heic"));

    let people: Vec<(&str, Option<&str>)> = metadata
        .people
        .iter()
        .map(|person| (person.name.as_str(), person.birth_date.as_deref()))
        .collect();
    assert_eq!(people, [("Ann", Some("1990-01-15")), ("Guest", None)]);

    let tags: Vec<&str> = metadata.tags.iter().map(|tag| tag.value.as_str()).collect();
    assert_eq!(tags, ["family", "beach"]);

    let exif = metadata.exif_info.expect("exif info is present");
    assert_eq!(exif.description.as_deref(), Some("Sunset on the Tagus"));
    assert_eq!(exif.city.as_deref(), Some("Lisbon"));
    assert_eq!(exif.state.as_deref(), Some("Lisboa"));
    assert_eq!(exif.country.as_deref(), Some("Portugal"));
    assert_eq!(exif.make.as_deref(), Some("Apple"));
    assert_eq!(exif.model.as_deref(), Some("iPhone 15 Pro"));
    assert_eq!(
        exif.date_time_original.as_deref(),
        Some("2024-04-30T18:22:00.000Z")
    );
    assert_eq!(
        exif.lens_model.as_deref(),
        Some("iPhone 15 Pro back camera")
    );
    assert_eq!(exif.exposure_time.as_deref(), Some("1/250"));
    assert_eq!(exif.f_number, Some(2.8_f64));
    assert_eq!(exif.focal_length, Some(35.4_f64));
    assert_eq!(exif.iso, Some(400));
    assert_eq!(exif.rating, Some(4));
    assert_eq!(exif.time_zone.as_deref(), Some("Europe/Lisbon"));
}

#[test]
fn missing_metadata_fields_fall_back_to_empty_values() {
    let metadata: AssetMetadata =
        serde_json::from_str(r#"{"originalFileName": "photo.jpg"}"#).expect("partial payload");

    assert_eq!(metadata.original_file_name.as_deref(), Some("photo.jpg"));
    assert!(metadata.r#type.is_none());
    assert!(metadata.file_created_at.is_none());
    assert!(metadata.local_date_time.is_none());
    assert!(metadata.width.is_none());
    assert!(metadata.height.is_none());
    assert!(metadata.original_mime_type.is_none());
    assert!(metadata.exif_info.is_none());
    assert!(metadata.people.is_empty());
    assert!(metadata.tags.is_empty());
}

#[test]
fn asset_response_parses_the_id_and_the_exif_description() {
    let asset: AssetResponse = serde_json::from_str(
        r#"{"id": "5a3f2b1c-0000-4000-8000-000000000001", "exifInfo": {"description": "Sunset"}}"#,
    )
    .expect("asset payload deserializes");

    assert_eq!(asset.id, "5a3f2b1c-0000-4000-8000-000000000001");
    assert_eq!(
        asset.exif_info.and_then(|exif| exif.description).as_deref(),
        Some("Sunset")
    );
}

#[test]
fn assets_without_exif_data_parse_without_exif_info() {
    let asset: AssetResponse =
        serde_json::from_str(r#"{"id": "5a3f2b1c-0000-4000-8000-000000000001"}"#)
            .expect("asset payload without exif deserializes");

    assert!(asset.exif_info.is_none());
}

#[test]
fn asset_payloads_without_an_id_are_rejected() {
    assert!(serde_json::from_str::<AssetResponse>(r#"{"exifInfo": null}"#).is_err());
}
