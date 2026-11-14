//! iOS stand-in for `arboard` 3.7 (upstream's Unix backend is X11/Wayland, which
//! doesn't build for iOS). Text and HTML go through `UIPasteboard.general`;
//! images and file lists report "not available" so apps fall back gracefully.
//! `common.rs` (Error, ImageData) is copied unchanged from arboard (MIT OR Apache-2.0).

#[allow(dead_code)]
mod common;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

pub use common::Error;
#[cfg(feature = "image-data")]
pub use common::ImageData;

pub struct Clipboard {
    _private: (),
}

impl Clipboard {
    pub fn new() -> Result<Self, Error> {
        Ok(Clipboard { _private: () })
    }
    pub fn get_text(&mut self) -> Result<String, Error> {
        self.get().text()
    }
    pub fn set_text<'a, T: Into<Cow<'a, str>>>(&mut self, text: T) -> Result<(), Error> {
        self.set().text(text)
    }
    pub fn set_html<'a, T: Into<Cow<'a, str>>>(&mut self, html: T, alt_text: Option<T>) -> Result<(), Error> {
        self.set().html(html, alt_text)
    }
    #[cfg(feature = "image-data")]
    pub fn get_image(&mut self) -> Result<ImageData<'static>, Error> {
        self.get().image()
    }
    #[cfg(feature = "image-data")]
    pub fn set_image(&mut self, image: ImageData) -> Result<(), Error> {
        self.set().image(image)
    }
    pub fn clear(&mut self) -> Result<(), Error> {
        self.clear_with().default()
    }
    pub fn clear_with(&mut self) -> Clear<'_> {
        Clear { _c: self }
    }
    pub fn get(&mut self) -> Get<'_> {
        Get { _c: self }
    }
    pub fn set(&mut self) -> Set<'_> {
        Set { _c: self }
    }
}

#[must_use]
pub struct Get<'clipboard> {
    _c: &'clipboard mut Clipboard,
}

impl Get<'_> {
    pub fn text(self) -> Result<String, Error> {
        platform::get_text().ok_or(Error::ContentNotAvailable)
    }
    #[cfg(feature = "image-data")]
    pub fn image(self) -> Result<ImageData<'static>, Error> {
        Err(Error::ContentNotAvailable)
    }
    pub fn html(self) -> Result<String, Error> {
        platform::get_html().ok_or(Error::ContentNotAvailable)
    }
    pub fn file_list(self) -> Result<Vec<PathBuf>, Error> {
        Err(Error::ContentNotAvailable)
    }
}

#[must_use]
pub struct Set<'clipboard> {
    _c: &'clipboard mut Clipboard,
}

impl Set<'_> {
    pub fn text<'a, T: Into<Cow<'a, str>>>(self, text: T) -> Result<(), Error> {
        platform::set_text(&text.into());
        Ok(())
    }
    pub fn html<'a, T: Into<Cow<'a, str>>>(self, html: T, alt_text: Option<T>) -> Result<(), Error> {
        let html = html.into();
        let alt = alt_text.map(|t| t.into());
        platform::set_html(&html, alt.as_deref());
        Ok(())
    }
    #[cfg(feature = "image-data")]
    pub fn image(self, _image: ImageData) -> Result<(), Error> {
        Err(Error::ClipboardNotSupported)
    }
    pub fn file_list(self, _file_list: &[impl AsRef<Path>]) -> Result<(), Error> {
        Err(Error::ClipboardNotSupported)
    }
}

#[must_use]
pub struct Clear<'clipboard> {
    _c: &'clipboard mut Clipboard,
}

impl Clear<'_> {
    pub fn default(self) -> Result<(), Error> {
        platform::set_text("");
        Ok(())
    }
}

#[cfg(target_os = "ios")]
mod platform {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSData, NSDictionary, NSString};
    use objc2_ui_kit::UIPasteboard;

    const HTML_UTI: &str = "public.html";
    const TEXT_UTI: &str = "public.utf8-plain-text";

    pub(crate) fn get_text() -> Option<String> {
        let pb = UIPasteboard::generalPasteboard();
        unsafe { pb.string() }.map(|s| s.to_string())
    }

    pub(crate) fn set_text(text: &str) {
        let pb = UIPasteboard::generalPasteboard();
        unsafe { pb.setString(Some(&NSString::from_str(text))) };
    }

    pub(crate) fn get_html() -> Option<String> {
        let pb = UIPasteboard::generalPasteboard();
        let data: Retained<NSData> = pb.dataForPasteboardType(&NSString::from_str(HTML_UTI))?;
        String::from_utf8(data.to_vec()).ok()
    }

    pub(crate) fn set_html(html: &str, alt: Option<&str>) {
        let pb = UIPasteboard::generalPasteboard();
        let html_data = NSData::with_bytes(html.as_bytes());
        let mut keys: Vec<Retained<NSString>> = vec![NSString::from_str(HTML_UTI)];
        let mut vals: Vec<Retained<AnyObject>> = vec![html_data.into()];
        if let Some(alt) = alt {
            keys.push(NSString::from_str(TEXT_UTI));
            vals.push(NSString::from_str(alt).into());
        }
        let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
        let item = NSDictionary::from_retained_objects(&key_refs, &vals);
        let items = objc2_foundation::NSArray::from_retained_slice(&[item]);
        unsafe { pb.setItems(&items) };
    }
}

#[cfg(not(target_os = "ios"))]
mod platform {
    pub(crate) fn get_text() -> Option<String> {
        None
    }
    pub(crate) fn set_text(_text: &str) {}
    pub(crate) fn get_html() -> Option<String> {
        None
    }
    pub(crate) fn set_html(_html: &str, _alt: Option<&str>) {}
}
