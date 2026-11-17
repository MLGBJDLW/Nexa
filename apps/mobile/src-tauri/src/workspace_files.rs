use cap_primitives::{ambient_authority, fs};
use std::{
    fs::File,
    io::{self, ErrorKind},
    path::{Component, Path, PathBuf},
};

pub const DIRECTORY: &str = "workspace";

pub fn open(app_data: &Path) -> io::Result<File> {
    let parent = fs::open_ambient_dir(app_data, ambient_authority())?;
    fs::open_dir_nofollow(&parent, Path::new(DIRECTORY))
}

// Hold the parent directory handle across inspection and unlink. A phone task
// can rename a directory while the file sheet is open.
fn open_directory(root: &File, relative: &Path) -> io::Result<File> {
    let mut directory = root.try_clone()?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Choose a phone file.",
            ));
        };
        directory = fs::open_dir_nofollow(&directory, Path::new(name))?;
    }
    Ok(directory)
}

pub fn list(root: &File) -> io::Result<Vec<String>> {
    let mut paths = Vec::new();
    let mut directories = vec![PathBuf::new()];
    while let Some(relative) = directories.pop() {
        let directory = match open_directory(root, &relative) {
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            result => result?,
        };
        for entry in fs::read_base_dir(&directory)? {
            let entry = entry?;
            // The UI returns this exact string for deletion. Lossy names can
            // collide with a different file that contains a literal U+FFFD.
            let path = relative.join(entry.file_name());
            let Ok(path) = path.into_os_string().into_string() else {
                continue;
            };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                directories.push(PathBuf::from(path));
            } else if kind.is_file() {
                paths.push(path);
            }
        }
    }
    paths.sort_unstable();
    Ok(paths)
}

pub fn delete(root: &File, path: &Path) -> io::Result<()> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Choose a phone file.",
        ));
    };
    let remove = || {
        let directory = open_directory(root, parent)?;
        let name = Path::new(name);
        if !fs::stat(&directory, name, fs::FollowSymlinks::No)?.is_file() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Choose a regular file.",
            ));
        }
        fs::remove_file(&directory, name)
    };
    // The listed file may already have been removed by a phone task or a retry.
    match remove() {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as host_fs;

    #[test]
    fn deletes_only_listed_regular_files_and_tolerates_retries() {
        let data = std::env::temp_dir().join(format!("nexa-files-{}", uuid::Uuid::new_v4()));
        let workspace = data.join(DIRECTORY);
        host_fs::create_dir_all(workspace.join("nested")).unwrap();
        let relative = Path::new("nested").join("résumé.txt");
        host_fs::write(workspace.join(&relative), "phone").unwrap();
        let private = data.join("private");
        host_fs::write(&private, "private").unwrap();
        let root = open(&data).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&private, workspace.join("link")).unwrap();
            std::os::unix::fs::symlink(&data, workspace.join("outside")).unwrap();
            assert!(delete(&root, Path::new("link")).is_err());
            assert!(delete(&root, Path::new("outside/private")).is_err());
        }
        assert_eq!(list(&root).unwrap(), vec![relative.to_str().unwrap()]);
        for forbidden in [Path::new("nested"), Path::new("../private"), &private] {
            assert!(delete(&root, forbidden).is_err());
        }
        delete(&root, &relative).unwrap();
        delete(&root, &relative).unwrap();
        assert!(list(&root).unwrap().is_empty());
        assert_eq!(host_fs::read(&private).unwrap(), b"private");
        drop(root);
        host_fs::remove_dir_all(data).unwrap();
    }
}
