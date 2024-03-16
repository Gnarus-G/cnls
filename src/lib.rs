use std::fmt::Debug;

pub mod scope;

pub type Array<T> = Box<[T]>;

#[repr(transparent)]
#[derive(PartialEq, Clone)]
pub struct Str(Box<str>);

impl Debug for Str {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl std::ops::Deref for Str {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl From<&str> for Str {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl PartialEq<str> for Str {
    fn eq(&self, other: &str) -> bool {
        return &*self.0 == other;
    }
}

pub mod fs {
    use std::{
        ffi::OsStr,
        path::{Path, PathBuf},
    };

    pub fn find_all_css_files_in_dir(
        dir: impl AsRef<Path>,
        css_files: &mut Vec<PathBuf>,
    ) -> anyhow::Result<()> {
        for d in ignore::Walk::new(dir) {
            match d {
                Ok(entry) => {
                    let path = entry.path();
                    let is_css_file = path.is_file()
                        && path
                            .extension()
                            .map(|e| e == OsStr::new("css"))
                            .unwrap_or(false);

                    if is_css_file {
                        css_files.push(path.to_path_buf());
                    }
                }
                Err(err) => eprintln!("[ERROR] failed to read a directory entry: {err}"),
            }
        }

        Ok(())
    }
}
