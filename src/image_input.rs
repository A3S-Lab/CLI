//! Shared validation and capability checks for user-supplied image input.

use std::io::{self, Cursor, Read};
use std::path::Path;

use a3s_code_core::config::CodeConfig;
use a3s_code_core::llm::Attachment;

use crate::model::route::{ModelRoute, ModelSource};

pub(crate) const MAX_IMAGE_COUNT: usize = 20;
pub(crate) const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
pub(crate) const MAX_IMAGE_BATCH_BYTES: u64 = 50 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_IMAGE_DECODE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct ValidatedImage {
    attachment: Attachment,
    width: u32,
    height: u32,
    extension: &'static str,
}

impl ValidatedImage {
    pub(crate) fn from_file(path: &Path) -> io::Result<Self> {
        let metadata = std::fs::metadata(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not inspect image {}: {error}", path.display()),
            )
        })?;
        if !metadata.is_file() {
            return Err(invalid_image(format!(
                "image path is not a regular file: {}",
                path.display()
            )));
        }
        if metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES {
            return Err(invalid_image(format!(
                "image {} must be between 1 byte and {} MiB",
                path.display(),
                MAX_IMAGE_BYTES / 1024 / 1024
            )));
        }

        let file = std::fs::File::open(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not open image {}: {error}", path.display()),
            )
        })?;
        let mut data = Vec::with_capacity(metadata.len().min(MAX_IMAGE_BYTES) as usize);
        file.take(MAX_IMAGE_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not read image {}: {error}", path.display()),
                )
            })?;
        if data.len() as u64 > MAX_IMAGE_BYTES {
            return Err(invalid_image(format!(
                "image {} grew beyond the {} MiB limit while it was read",
                path.display(),
                MAX_IMAGE_BYTES / 1024 / 1024
            )));
        }
        Self::from_bytes(data, &path.display().to_string())
    }

    pub(crate) fn from_bytes(data: Vec<u8>, label: &str) -> io::Result<Self> {
        if data.is_empty() || data.len() as u64 > MAX_IMAGE_BYTES {
            return Err(invalid_image(format!(
                "{label} must be between 1 byte and {} MiB",
                MAX_IMAGE_BYTES / 1024 / 1024
            )));
        }
        let format = image::guess_format(&data).map_err(|error| {
            invalid_image(format!("{label} is not a recognized image: {error}"))
        })?;
        let (media_type, extension) = supported_format(format).ok_or_else(|| {
            invalid_image(format!(
                "{label} uses an unsupported image format; expected PNG, JPEG, GIF, or WebP"
            ))
        })?;
        let decoded = decode(&data, format, label)?;
        Ok(Self {
            attachment: Attachment::new(data, media_type),
            width: decoded.width(),
            height: decoded.height(),
            extension,
        })
    }

    pub(crate) fn normalized_png(data: &[u8], label: &str) -> io::Result<Self> {
        if data.is_empty() || data.len() as u64 > MAX_IMAGE_BYTES {
            return Err(invalid_image(format!(
                "{label} must be between 1 byte and {} MiB",
                MAX_IMAGE_BYTES / 1024 / 1024
            )));
        }
        let format = image::guess_format(data).map_err(|error| {
            invalid_image(format!("{label} is not a recognized image: {error}"))
        })?;
        if supported_format(format).is_none() {
            return Err(invalid_image(format!(
                "{label} uses an unsupported image format; expected PNG, JPEG, GIF, or WebP"
            )));
        }
        let decoded = decode(data, format, label)?;
        let (width, height) = (decoded.width(), decoded.height());
        let mut encoded = Cursor::new(Vec::new());
        decoded
            .write_to(&mut encoded, image::ImageFormat::Png)
            .map_err(|error| {
                invalid_image(format!("{label} could not be encoded as PNG: {error}"))
            })?;
        let data = encoded.into_inner();
        if data.len() as u64 > MAX_IMAGE_BYTES {
            return Err(invalid_image(format!(
                "normalized {label} exceeds the {} MiB image limit",
                MAX_IMAGE_BYTES / 1024 / 1024
            )));
        }
        Ok(Self {
            attachment: Attachment::png(data),
            width,
            height,
            extension: "png",
        })
    }

    pub(crate) fn attachment(&self) -> &Attachment {
        &self.attachment
    }

    pub(crate) fn into_attachment(self) -> Attachment {
        self.attachment
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub(crate) fn extension(&self) -> &'static str {
        self.extension
    }

    pub(crate) fn byte_len(&self) -> u64 {
        self.attachment.data.len() as u64
    }
}

pub(crate) fn load_image_attachments(paths: &[std::path::PathBuf]) -> io::Result<Vec<Attachment>> {
    ensure_batch_limits(paths.len(), 0)?;
    let mut total_bytes = 0_u64;
    let mut attachments = Vec::with_capacity(paths.len());
    for path in paths {
        let image = ValidatedImage::from_file(path)?;
        total_bytes = total_bytes.saturating_add(image.byte_len());
        ensure_batch_limits(attachments.len() + 1, total_bytes)?;
        attachments.push(image.into_attachment());
    }
    Ok(attachments)
}

pub(crate) fn ensure_batch_limits(image_count: usize, total_bytes: u64) -> io::Result<()> {
    if image_count > MAX_IMAGE_COUNT {
        return Err(invalid_image(format!(
            "at most {MAX_IMAGE_COUNT} images may be attached to one turn"
        )));
    }
    if total_bytes > MAX_IMAGE_BATCH_BYTES {
        return Err(invalid_image(format!(
            "image attachments exceed the {} MiB per-turn limit",
            MAX_IMAGE_BATCH_BYTES / 1024 / 1024
        )));
    }
    Ok(())
}

pub(crate) fn ensure_model_supports_images(
    config: &CodeConfig,
    model_ref: Option<&str>,
) -> anyhow::Result<()> {
    let model_ref = model_ref
        .or(config.default_model.as_deref())
        .ok_or_else(|| anyhow::anyhow!("an image-capable model must be selected"))?;
    let route = model_ref.parse::<ModelRoute>()?;
    ensure_route_supports_images(config, &route)
}

pub(crate) fn ensure_active_model_supports_images(
    config: &CodeConfig,
    source: ModelSource,
    model: Option<&str>,
) -> anyhow::Result<()> {
    let model = model
        .or(config.default_model.as_deref())
        .ok_or_else(|| anyhow::anyhow!("an image-capable model must be selected"))?;
    let route = if source == ModelSource::Config {
        model.parse::<ModelRoute>()?
    } else {
        ModelRoute::new(source, model)?
    };
    ensure_route_supports_images(config, &route)
}

fn ensure_route_supports_images(config: &CodeConfig, route: &ModelRoute) -> anyhow::Result<()> {
    match route.source {
        ModelSource::Codex | ModelSource::Claude | ModelSource::OsGateway => Ok(()),
        ModelSource::Kimi | ModelSource::CodeBuddy => anyhow::bail!(
            "{} account transport cannot carry image input; select Codex, Claude, A3S OS, or an image-capable config model",
            route.source.label()
        ),
        ModelSource::Config => {
            let (provider_name, model_id) = route.model.split_once('/').ok_or_else(|| {
                anyhow::anyhow!("configured model route must use provider/model format")
            })?;
            let provider = config.find_provider(provider_name).ok_or_else(|| {
                anyhow::anyhow!("configured provider `{provider_name}` was not found")
            })?;
            let model = provider.find_model(model_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "configured model `{}` was not found for provider `{provider_name}`",
                    model_id
                )
            })?;
            let image_modality = model
                .modalities
                .input
                .iter()
                .any(|modality| modality.eq_ignore_ascii_case("image"));
            if model.attachment || image_modality {
                Ok(())
            } else {
                anyhow::bail!(
                    "model `{}` is not configured for image input; set `attachment = true` or include `image` in `modalities.input`",
                    route.model
                )
            }
        }
    }
}

fn decode(data: &[u8], format: image::ImageFormat, label: &str) -> io::Result<image::DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(data));
    reader.set_format(format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| invalid_image(format!("{label} could not be decoded safely: {error}")))?;
    if decoded.width() == 0 || decoded.height() == 0 {
        return Err(invalid_image(format!("{label} has no pixels")));
    }
    Ok(decoded)
}

fn supported_format(format: image::ImageFormat) -> Option<(&'static str, &'static str)> {
    match format {
        image::ImageFormat::Png => Some(("image/png", "png")),
        image::ImageFormat::Jpeg => Some(("image/jpeg", "jpg")),
        image::ImageFormat::Gif => Some(("image/gif", "gif")),
        image::ImageFormat::WebP => Some(("image/webp", "webp")),
        _ => None,
    }
}

fn invalid_image(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            width,
            height,
            image::Rgb([10, 20, 30]),
        ));
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        encoded.into_inner()
    }

    #[test]
    fn file_validation_uses_content_not_a_spoofable_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("screen.jpg");
        std::fs::write(&path, png_bytes(7, 5)).unwrap();

        let image = ValidatedImage::from_file(&path).unwrap();

        assert_eq!(image.attachment().media_type, "image/png");
        assert_eq!(image.dimensions(), (7, 5));
        assert_eq!(image.extension(), "png");
    }

    #[test]
    fn file_validation_rejects_non_image_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("screen.png");
        std::fs::write(&path, b"not an image").unwrap();

        let error = ValidatedImage::from_file(&path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("not a recognized image"));
    }

    #[test]
    fn batch_limits_are_fail_closed() {
        assert!(ensure_batch_limits(MAX_IMAGE_COUNT, MAX_IMAGE_BATCH_BYTES).is_ok());
        assert!(ensure_batch_limits(MAX_IMAGE_COUNT + 1, 0).is_err());
        assert!(ensure_batch_limits(1, MAX_IMAGE_BATCH_BYTES + 1).is_err());
    }

    #[test]
    fn configured_model_must_advertise_image_input() {
        let supported = CodeConfig::from_acl(
            r#"
default_model = "openai/vision"
providers "openai" {
  apiKey = "test"
  models "vision" { attachment = true }
}
"#,
        )
        .unwrap();
        let unsupported = CodeConfig::from_acl(
            r#"
default_model = "openai/text"
providers "openai" {
  apiKey = "test"
  models "text" { modalities = { input = ["text"], output = ["text"] } }
}
"#,
        )
        .unwrap();

        assert!(ensure_model_supports_images(&supported, None).is_ok());
        assert!(ensure_model_supports_images(&unsupported, None)
            .unwrap_err()
            .to_string()
            .contains("not configured for image input"));
    }
}
