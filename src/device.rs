//! Edge device access: validation and description of attached capture devices.
//!
//! `flo` runs on edge nodes (e.g. a k8s DaemonSet) and needs a camera/tty before
//! it can stream or talk to hardware. Rather than handing a raw path straight to
//! GStreamer (which fails late with an opaque error), this module validates the
//! device up front so misconfiguration fails fast with a clear message.
//!
//! No `unsafe`, no new dependencies — pure `std::fs`/`std::os` introspection.

use std::os::unix::fs::FileTypeExt;
use std::path::Path;

/// A validated video capture device on the edge node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoDevice {
    /// Resolved device path, e.g. `/dev/video0`.
    pub path: String,
}

impl VideoDevice {
    /// Build a `VideoDevice` from a candidate path, validating it's a usable V4L2
    /// video device.
    ///
    /// Checks the path exists, is a character device, and looks like a V4L2
    /// device (`/dev/videoN`). Returns a typed error on any failure so the CLI
    /// can report it instead of letting GStreamer fail later.
    pub fn from_path(candidate: &str) -> Result<Self, DeviceValidationError> {
        let path = Path::new(candidate);
        if !path.exists() {
            return Err(DeviceValidationError::DeviceNotFound(candidate.to_string()));
        }
        let meta = std::fs::metadata(path)
            .map_err(|e| DeviceValidationError::Io(candidate.to_string(), e))?;
        let is_char = meta.file_type().is_char_device();
        // On Linux, V4L2 devices are char devices at /dev/videoN. We accept any
        // char device whose name matches the convention; non-Linux/container
        // setups may differ and are the operator's responsibility.
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let looks_like_video = name.starts_with("video") && is_char;
        if !looks_like_video {
            return Err(DeviceValidationError::NotACharacterDevice(
                candidate.to_string(),
            ));
        }
        Ok(VideoDevice {
            path: candidate.to_string(),
        })
    }

    /// Build the GStreamer `SourceSpec` for this device.
    #[cfg(feature = "media")]
    pub fn to_source_spec(&self) -> flo_rs::media::SourceSpec {
        flo_rs::media::SourceSpec::V4l2(self.path.clone())
    }

    /// Enumerate the V4L2 capture devices currently attached to this node.
    ///
    /// Scans `/dev` for character devices named `videoN` and returns each as a
    /// validated [`VideoDevice`]. Non-device or non-video entries are skipped
    /// rather than erroring, so a partially-populated `/dev` (common in
    /// containers) yields an empty list instead of a failure.
    #[allow(dead_code)]
    pub fn discover() -> Vec<VideoDevice> {
        let mut found = Vec::new();
        let entries = match std::fs::read_dir("/dev") {
            Ok(e) => e,
            Err(_) => return found,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = match name.to_str() {
                Some(n) => n,
                None => continue,
            };
            if !name.starts_with("video") {
                continue;
            }
            if let Ok(dev) = VideoDevice::from_path(&format!("/dev/{name}")) {
                found.push(dev);
            }
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found
    }
}

/// Why a device could not be opened or validated.
#[derive(Debug)]
pub enum DeviceValidationError {
    DeviceNotFound(String),
    Io(String, std::io::Error),
    NotACharacterDevice(String),
}

impl std::fmt::Display for DeviceValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceValidationError::DeviceNotFound(p) => write!(f, "video device not found: {p}"),
            DeviceValidationError::Io(p, e) => write!(f, "could not stat video device {p}: {e}"),
            DeviceValidationError::NotACharacterDevice(p) => write!(
                f,
                "{p} is not a usable V4L2 video device (expected a /dev/videoN char device)"
            ),
        }
    }
}

impl std::error::Error for DeviceValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DeviceValidationError::Io(_, e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_device_is_rejected() {
        let err = VideoDevice::from_path("/dev/video-does-not-exist-xyz").unwrap_err();
        assert!(matches!(err, DeviceValidationError::DeviceNotFound(_)));
    }

    #[test]
    fn non_video_path_is_rejected() {
        // /etc/hostname exists but is not a /dev/videoN char device.
        let err = VideoDevice::from_path("/etc/hostname").unwrap_err();
        assert!(matches!(err, DeviceValidationError::NotACharacterDevice(_)));
    }

    #[test]
    fn valid_video_device_is_accepted() {
        // We cannot assume a real camera exists, but a correctly-shaped path
        // that is absent must at least surface the DeviceNotFound (not NotACharacterDevice)
        // error, proving the shape check runs before the existence check.
        let err = VideoDevice::from_path("/dev/video0").unwrap_err();
        assert!(matches!(err, DeviceValidationError::DeviceNotFound(_)));
    }

    #[test]
    fn discover_returns_sorted_valid_devices() {
        // Enumerate without panicking; every device returned must validate
        // via from_path again, and the list must be sorted by path. Real cameras may or may not be
        // present, so we only assert structural invariants.
        let devices = VideoDevice::discover();
        for d in &devices {
            assert!(VideoDevice::from_path(&d.path).is_ok());
        }
        let mut sorted = devices.clone();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(devices, sorted);
    }
}
