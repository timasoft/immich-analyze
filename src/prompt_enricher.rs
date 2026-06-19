use crate::{config::ProcessingContext, data_access::DataAccess};
use chrono::{Datelike, NaiveDate};
use log::warn;
use uuid::Uuid;

pub struct PromptContext {
    pub base_prompt: String,
    pub created_at: Option<String>,
    pub location: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_model: Option<String>,
    pub exif_description: Option<String>,
    pub exposure_time: Option<String>,
    pub f_number: Option<f64>,
    pub focal_length: Option<f64>,
    pub iso: Option<u32>,
    pub rating: Option<u8>,
    pub time_zone: Option<String>,
    pub original_file_name: Option<String>,
    pub asset_type: Option<String>,
    pub people: Vec<(String, Option<u32>)>,
    pub tags: Vec<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub mime_type: Option<String>,
}

impl PromptContext {
    pub fn new(base_prompt: &str) -> Self {
        Self {
            base_prompt: base_prompt.to_string(),
            created_at: None,
            location: None,
            camera_make: None,
            camera_model: None,
            lens_model: None,
            exif_description: None,
            exposure_time: None,
            f_number: None,
            focal_length: None,
            iso: None,
            rating: None,
            time_zone: None,
            original_file_name: None,
            asset_type: None,
            people: Vec::new(),
            tags: Vec::new(),
            width: None,
            height: None,
            mime_type: None,
        }
    }

    pub fn with_created_at(mut self, date: Option<String>) -> Self {
        self.created_at = date;
        self
    }

    pub fn with_location(mut self, location: Option<String>) -> Self {
        self.location = location;
        self
    }

    pub fn with_camera_info(mut self, make: Option<String>, model: Option<String>) -> Self {
        self.camera_make = make;
        self.camera_model = model;
        self
    }

    pub fn with_exif_description(mut self, desc: Option<String>) -> Self {
        self.exif_description = desc;
        self
    }

    pub fn with_lens_model(mut self, lens: Option<String>) -> Self {
        self.lens_model = lens;
        self
    }

    pub fn with_exposure_settings(
        mut self,
        exposure_time: Option<String>,
        f_number: Option<f64>,
        focal_length: Option<f64>,
        iso: Option<u32>,
    ) -> Self {
        self.exposure_time = exposure_time;
        self.f_number = f_number;
        self.focal_length = focal_length;
        self.iso = iso;
        self
    }

    pub const fn with_rating(mut self, rating: Option<u8>) -> Self {
        self.rating = rating;
        self
    }

    pub fn with_time_zone(mut self, tz: Option<String>) -> Self {
        self.time_zone = tz;
        self
    }

    pub fn with_file_info(
        mut self,
        original_file_name: Option<String>,
        asset_type: Option<String>,
    ) -> Self {
        self.original_file_name = original_file_name;
        self.asset_type = asset_type;
        self
    }

    pub fn with_people(mut self, people: Vec<(String, Option<u32>)>) -> Self {
        self.people = people;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_resolution(mut self, width: Option<i32>, height: Option<i32>) -> Self {
        self.width = width.map(|w| w as u32);
        self.height = height.map(|h| h as u32);
        self
    }

    pub fn with_mime_type(mut self, mime: Option<String>) -> Self {
        self.mime_type = mime;
        self
    }

    pub fn build_enriched_prompt(&self) -> String {
        let mut context_parts = Vec::new();

        if let Some(ref asset_type) = self.asset_type {
            let desc = match self.mime_type.as_deref() {
                Some("image/dng") => "RAW photo (DNG)".to_string(),
                Some(mime) if mime.starts_with("image/") => {
                    format!("{} photo", mime[6..].to_uppercase())
                }
                Some(mime) if mime.starts_with("video/") => {
                    format!("{} video", mime[6..].to_uppercase())
                }
                _ => asset_type.clone(),
            };
            context_parts.push(format!("Asset type: {desc}"));
        }

        if let (Some(w), Some(h)) = (self.width, self.height) {
            context_parts.push(format!("Resolution: {w}×{h}"));
        }

        if let Some(ref date) = self.created_at {
            context_parts.push(format!("Date taken: {date}"));
        }

        if let Some(ref tz) = self.time_zone {
            context_parts.push(format!("Time zone: {tz}"));
        }

        if let Some(ref location) = self.location {
            context_parts.push(format!("Location: {location}"));
        }

        if !self.people.is_empty() {
            let people_str: Vec<String> = self
                .people
                .iter()
                .map(|(name, age)| match age {
                    Some(a) if *a == 0 => format!("{name} (<1 year)"),
                    Some(a) => format!("{name} ({a} years)"),
                    None => name.clone(),
                })
                .collect();
            context_parts.push(format!("People: {}", people_str.join(", ")));
        }

        if !self.tags.is_empty() {
            context_parts.push(format!("Tags: {}", self.tags.join(", ")));
        }

        if let Some(ref make) = self.camera_make {
            let camera_info = if let Some(ref model) = self.camera_model {
                format!("{make} {model}")
            } else {
                make.clone()
            };
            context_parts.push(format!("Camera: {camera_info}"));
        }

        if let Some(ref lens) = self.lens_model {
            context_parts.push(format!("Lens: {lens}"));
        }

        let mut exposure_parts = Vec::new();
        if let Some(ref et) = self.exposure_time {
            exposure_parts.push(format!("{et}s"));
        }
        if let Some(f) = self.f_number {
            exposure_parts.push(format!("f/{f:.1}"));
        }
        if let Some(fl) = self.focal_length {
            exposure_parts.push(format!("{fl:.0}mm"));
        }
        if let Some(iso) = self.iso {
            exposure_parts.push(format!("ISO {iso}"));
        }
        if !exposure_parts.is_empty() {
            context_parts.push(format!("Exposure: {}", exposure_parts.join(", ")));
        }

        if let Some(rating) = self.rating {
            context_parts.push(format!("Rating: {rating}/5"));
        }

        if let Some(ref name) = self.original_file_name {
            context_parts.push(format!("Original filename: {name}"));
        }

        if let Some(ref desc) = self.exif_description
            && !desc.is_empty()
        {
            context_parts.push(format!("Existing description: {desc}"));
        }

        if context_parts.is_empty() {
            self.base_prompt.clone()
        } else {
            let context = context_parts.join("\n");
            format!("{}\n\nAdditional context:\n{}", self.base_prompt, context)
        }
    }
}

/// Computes age at the time the photo was taken.
fn calculate_age(birth_date: Option<&str>, photo_date: Option<&str>) -> Option<u32> {
    let birth = NaiveDate::parse_from_str(birth_date?, "%Y-%m-%d").ok()?;
    let photo_str = photo_date?;
    let photo = if photo_str.len() >= 10 {
        NaiveDate::parse_from_str(&photo_str[..10], "%Y-%m-%d").ok()?
    } else {
        return None;
    };

    let age_years = photo.year() - birth.year();
    if age_years < 0 {
        return None;
    }
    let birthday_this_year = birth.with_year(photo.year())?;
    if photo < birthday_this_year {
        Some((age_years - 1) as u32)
    } else {
        Some(age_years as u32)
    }
}

pub async fn enrich_prompt_if_needed(
    ctx: &ProcessingContext<'_>,
    asset_id: &Uuid,
) -> Option<String> {
    if !ctx.enrich_prompt {
        return None;
    }

    if let DataAccess::ImmichApi { provider } = ctx.data_access {
        match provider.get_asset_metadata(asset_id).await {
            Ok(metadata) => {
                let photo_date = metadata
                    .local_date_time
                    .as_deref()
                    .or(metadata.file_created_at.as_deref())
                    .or_else(|| {
                        metadata
                            .exif_info
                            .as_ref()
                            .and_then(|e| e.date_time_original.as_deref())
                    });

                let people_with_ages: Vec<(String, Option<u32>)> = metadata
                    .people
                    .iter()
                    .map(|p| {
                        let age = calculate_age(p.birth_date.as_deref(), photo_date);
                        (p.name.clone(), age)
                    })
                    .collect();

                let tag_values: Vec<String> =
                    metadata.tags.iter().map(|t| t.value.clone()).collect();

                let mut context = PromptContext::new(ctx.prompt)
                    .with_file_info(metadata.original_file_name, metadata.r#type)
                    .with_people(people_with_ages)
                    .with_tags(tag_values)
                    .with_resolution(metadata.width, metadata.height)
                    .with_mime_type(metadata.original_mime_type);

                if let Some(exif) = metadata.exif_info {
                    let created_at = exif.date_time_original.or(metadata.file_created_at);
                    context = context.with_created_at(created_at);

                    let location_parts: Vec<String> = [exif.city, exif.state, exif.country]
                        .into_iter()
                        .flatten()
                        .filter(|s| !s.is_empty())
                        .collect();

                    let location = if location_parts.is_empty() {
                        None
                    } else {
                        Some(location_parts.join(", "))
                    };

                    context = context
                        .with_location(location)
                        .with_camera_info(exif.make, exif.model)
                        .with_lens_model(exif.lens_model)
                        .with_exposure_settings(
                            exif.exposure_time,
                            exif.f_number,
                            exif.focal_length,
                            exif.iso,
                        )
                        .with_rating(exif.rating)
                        .with_time_zone(exif.time_zone)
                        .with_exif_description(exif.description);
                } else {
                    context = context.with_created_at(metadata.file_created_at);
                }

                Some(context.build_enriched_prompt())
            }
            Err(e) => {
                warn!("Failed to get asset metadata for enrichment: {e}");
                None
            }
        }
    } else {
        warn!("Prompt enrichment is only supported in Immich API mode, skipping");
        None
    }
}
