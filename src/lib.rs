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
        for d in dir.as_ref().read_dir()? {
            match d {
                Ok(entry) if entry.path().is_dir() => {
                    find_all_css_files_in_dir(entry.path(), css_files)?
                }
                Ok(entry) => {
                    let path = entry.path();
                    assert!(path.is_file());
                    if path
                        .extension()
                        .map(|e| e == OsStr::new("css"))
                        .unwrap_or(false)
                    {
                        css_files.push(path);
                    }
                }
                Err(err) => eprintln!("[ERROR] failed to read a directory entry: {err}"),
            }
        }

        Ok(())
    }
}
