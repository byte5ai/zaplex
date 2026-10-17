use super::*;

#[test]
fn test_to_relative_path() {
    use super::to_relative_path;
    use std::path::Path;

    // Basic relative path conversion
    #[cfg(not(windows))]
    {
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john/projects/app/src/main.rs"),
                Path::new("/Users/john/projects")
            ),
            Some("app/src/main.rs".to_string())
        );

        // Same directory
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john/projects"),
                Path::new("/Users/john/projects")
            ),
            Some(".".to_string())
        );

        // Parent directory
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john"),
                Path::new("/Users/john/projects")
            ),
            Some("..".to_string())
        );

        // Nested parent
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users"),
                Path::new("/Users/john/projects")
            ),
            Some("../..".to_string())
        );

        // Cross-branch paths
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john/documents/file.txt"),
                Path::new("/Users/john/projects")
            ),
            Some("../documents/file.txt".to_string())
        );

        // Root to subdirectory
        assert_eq!(
            to_relative_path(false, Path::new("/var/log/system.log"), Path::new("/")),
            Some("var/log/system.log".to_string())
        );

        // Handles paths that would have leading slashes correctly
        assert_eq!(
            to_relative_path(false, Path::new("/home/user/file.txt"), Path::new("/home")),
            Some("user/file.txt".to_string())
        );

        // Test with current directory references that should be cleaned
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john/projects/./app/src/main.rs"),
                Path::new("/Users/john/projects")
            ),
            Some("app/src/main.rs".to_string()),
        );
    }

    #[cfg(windows)]
    {
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john/projects/app/src/main.rs"),
                Path::new("/Users/john/projects")
            ),
            Some("app\\src\\main.rs".to_string())
        );

        // Same directory
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john/projects"),
                Path::new("/Users/john/projects")
            ),
            Some(".".to_string())
        );

        // Parent directory
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john"),
                Path::new("/Users/john/projects")
            ),
            Some("..".to_string())
        );

        // Nested parent
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users"),
                Path::new("/Users/john/projects")
            ),
            Some("..\\..".to_string())
        );

        // Cross-branch paths
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john/documents/file.txt"),
                Path::new("/Users/john/projects")
            ),
            Some("..\\documents\\file.txt".to_string())
        );

        // Root to subdirectory
        assert_eq!(
            to_relative_path(false, Path::new("/var/log/system.log"), Path::new("/")),
            Some("var\\log\\system.log".to_string())
        );

        // Handles paths that would have leading slashes correctly
        assert_eq!(
            to_relative_path(false, Path::new("/home/user/file.txt"), Path::new("/home")),
            Some("user\\file.txt".to_string())
        );

        // Test with current directory references that should be cleaned
        assert_eq!(
            to_relative_path(
                false,
                Path::new("/Users/john/projects/./app/src/main.rs"),
                Path::new("/Users/john/projects")
            ),
            Some("app\\src\\main.rs".to_string())
        );

        // Windows paths - different drives should return None
        assert_eq!(
            to_relative_path(
                /* is_wsl */ false,
                Path::new("D:\\projects\\app"),
                Path::new("C:\\workspace")
            ),
            None,
        );

        // Windows paths - same drive
        assert_eq!(
            to_relative_path(
                /* is_wsl */ false,
                Path::new("C:\\projects\\app\\src\\main.rs"),
                Path::new("C:\\projects")
            ),
            Some("app\\src\\main.rs".to_string())
        );

        // Windows paths - same drive -- WSL is disabled for now
        assert_eq!(
            to_relative_path(
                /* is_wsl */ true,
                Path::new("C:\\projects\\app\\src\\main.rs"),
                Path::new("C:\\projects")
            ),
            None
        );
    }
}

#[test]
fn test_normalize_relative_path_for_glob() {
    use std::path::Path;

    assert_eq!(
        normalize_relative_path_for_glob(Path::new("app/src/main.rs")),
        "app/src/main.rs"
    );
    assert_eq!(
        normalize_relative_path_for_glob(Path::new("./app/src/main.rs")),
        "app/src/main.rs"
    );
    assert_eq!(
        normalize_relative_path_for_glob(Path::new("../app/src/main.rs")),
        "app/src/main.rs"
    );
    assert_eq!(normalize_relative_path_for_glob(Path::new("..")), "");
    assert_eq!(normalize_relative_path_for_glob(Path::new("")), "");
}

#[test]
fn test_posix_unescape() {
    let shell_family = ShellFamily::Posix;
    // Escaped spaces
    assert_eq!(shell_family.unescape("my\\ file.txt"), "my file.txt");
    // Multiple escaped characters
    assert_eq!(
        shell_family.unescape("path/to/my\\ file\\ \\(1\\).txt"),
        "path/to/my file (1).txt"
    );
    // No escaping needed — returns borrowed
    assert!(matches!(
        shell_family.unescape("simple.txt"),
        std::borrow::Cow::Borrowed(_)
    ));
    // Trailing backslash kept as-is
    assert_eq!(shell_family.unescape("trailing\\"), "trailing\\");
    // Roundtrip: unescape(escape(x)) == x
    let original = "hello world $HOME 'quotes'";
    assert_eq!(
        shell_family.unescape(&shell_family.escape(original)),
        original
    );
}

#[test]
fn test_powershell_unescape() {
    let shell_family = ShellFamily::PowerShell;
    // Escaped spaces
    assert_eq!(shell_family.unescape("my` file.txt"), "my file.txt");
    // Multiple escaped characters
    assert_eq!(shell_family.unescape("path` `$var"), "path $var");
    // No escaping needed — returns borrowed
    assert!(matches!(
        shell_family.unescape("simple.txt"),
        std::borrow::Cow::Borrowed(_)
    ));
    // Roundtrip: unescape(escape(x)) == x
    let original = "hello world $HOME";
    assert_eq!(
        shell_family.unescape(&shell_family.escape(original)),
        original
    );
}

#[test]
#[cfg(windows)]
fn test_msys2_exe_to_root() {
    assert_eq!(
        msys2_exe_to_root(WindowsPath::new(r"D:\Program Files\Git\usr\bin\git.exe")),
        WindowsPathBuf::from(r"D:\Program Files\Git")
    );
    assert_eq!(
        msys2_exe_to_root(WindowsPath::new(r"C:\git.exe")),
        WindowsPathBuf::from(r"C:\Program Files\Git")
    );
    assert_eq!(
        msys2_exe_to_root(WindowsPath::new(r"C:\foo\bar\baz\git.exe")),
        WindowsPathBuf::from(r"C:\Program Files\Git")
    );
    assert_eq!(
        msys2_exe_to_root(WindowsPath::new(r"C:\msys64\usr\bin\fish.exe")),
        WindowsPathBuf::from(r"C:\msys64")
    );
}

/// These tests all fail when running on UNIX.
#[test]
#[cfg(windows)]
fn test_convert_git_bash_to_windows_native_path() {
    use std::sync::LazyLock;
    static GIT_BASH_ROOT: LazyLock<WindowsPathBuf> =
        LazyLock::new(|| WindowsPathBuf::from(r"C:\Program Files\Git"));

    assert_eq!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("/c/foo/bar").to_path(),
            &GIT_BASH_ROOT
        )
        .unwrap(),
        PathBuf::from(r"C:\foo\bar")
    );
    assert_eq!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("/d/special folder").to_path(),
            &GIT_BASH_ROOT
        )
        .unwrap(),
        PathBuf::from(r"D:\special folder")
    );
    assert_eq!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("/z").to_path(),
            &GIT_BASH_ROOT
        )
        .unwrap(),
        PathBuf::from(r"Z:\")
    );
    // non-ascii isn't actually a valid drive name
    assert!(matches!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("/😊/invalid").to_path(),
            &GIT_BASH_ROOT
        ),
        Err(MSYS2PathConversionError::NotInDrive)
    ));
    assert!(matches!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("/aa/invalid").to_path(),
            &GIT_BASH_ROOT
        ),
        Err(MSYS2PathConversionError::NotInDrive)
    ));
    assert_eq!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("//wsl$/Ubuntu/home").to_path(),
            &GIT_BASH_ROOT
        )
        .unwrap(),
        PathBuf::from(r"\\wsl$\Ubuntu\home")
    );
    assert_eq!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("//WSL.localhost/Ubuntu/home").to_path(),
            &GIT_BASH_ROOT
        )
        .unwrap(),
        PathBuf::from(r"\\WSL.localhost\Ubuntu\home")
    );
    // This path might get auto-inferred by typed-path to be a Windows path, even if it looks like
    // UNIX with forward slashes.
    assert_eq!(
        convert_msys2_to_windows_native_path(
            &TypedPath::from("//wsl$/Ubuntu/home"),
            &GIT_BASH_ROOT
        )
        .unwrap(),
        PathBuf::from(r"\\wsl$\Ubuntu\home")
    );
    assert_eq!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("~/.bash_history").to_path(),
            &GIT_BASH_ROOT
        )
        .unwrap(),
        PathBuf::from(r"~\.bash_history")
    );
    // Relative paths cannot be converted.
    assert!(matches!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("some/relative/path").to_path(),
            &GIT_BASH_ROOT
        ),
        Err(MSYS2PathConversionError::PathNotAbsolute)
    ));
    assert!(matches!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_windows(r"C:\Users").to_path(),
            &GIT_BASH_ROOT
        ),
        Err(MSYS2PathConversionError::NonUnixPath)
    ));
    assert_eq!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("/").to_path(),
            &GIT_BASH_ROOT
        )
        .unwrap(),
        PathBuf::from(r"C:\Program Files\Git")
    );
    assert_eq!(
        convert_msys2_to_windows_native_path(
            &TypedPathBuf::from_unix("/usr/bin").to_path(),
            &GIT_BASH_ROOT
        )
        .unwrap(),
        PathBuf::from(r"C:\Program Files\Git\usr\bin")
    );
}

/// These tests all fail when running on UNIX.
#[test]
#[cfg(windows)]
fn test_convert_wsl_to_windows_host_path() {
    assert_eq!(
        convert_wsl_to_windows_host_path(
            &TypedPathBuf::from_unix("/mnt/c/foo/bar").to_path(),
            "Ubuntu"
        )
        .unwrap(),
        PathBuf::from(r"C:\foo\bar")
    );
    assert_eq!(
        convert_wsl_to_windows_host_path(
            &TypedPathBuf::from_unix("/mnt/e/special dir").to_path(),
            "Ubuntu"
        )
        .unwrap(),
        PathBuf::from(r"E:\special dir")
    );
    assert_eq!(
        convert_wsl_to_windows_host_path(&TypedPathBuf::from_unix("/mnt/z").to_path(), "Ubuntu")
            .unwrap(),
        PathBuf::from(r"Z:\")
    );
    assert_eq!(
        convert_wsl_to_windows_host_path(
            &TypedPathBuf::from_unix("/home/andy").to_path(),
            "Ubuntu"
        )
        .unwrap(),
        PathBuf::from(r"\\WSL$\Ubuntu\home\andy")
    );
    assert!(matches!(
        convert_wsl_to_windows_host_path(
            &TypedPathBuf::from_unix("some/relative/path").to_path(),
            "Ubuntu"
        ),
        Err(WSLPathConversionError::PathNotAbsolute)
    ));
    assert!(matches!(
        convert_wsl_to_windows_host_path(
            &TypedPathBuf::from_unix("~/.bash_history").to_path(),
            "Ubuntu"
        ),
        Err(WSLPathConversionError::PathNotAbsolute)
    ));
    // Two letters isn't actually a valid drive.
    assert_eq!(
        convert_wsl_to_windows_host_path(
            &TypedPathBuf::from_unix("/mnt/aa/invalid_drive").to_path(),
            "Ubuntu"
        )
        .unwrap(),
        PathBuf::from(r"\\WSL$\Ubuntu\mnt\aa\invalid_drive")
    );
    assert_eq!(
        convert_wsl_to_windows_host_path(
            &TypedPathBuf::from_unix("/mnt/😊/invalid_drive").to_path(),
            "Ubuntu"
        )
        .unwrap(),
        PathBuf::from(r"\\WSL$\Ubuntu\mnt\😊\invalid_drive")
    );
    assert!(matches!(
        convert_wsl_to_windows_host_path(
            &TypedPathBuf::from_windows(r"C:\Users").to_path(),
            "Ubuntu"
        ),
        Err(WSLPathConversionError::NonUnixPath)
    ));
}

#[test]
fn test_convert_windows_path_to_wsl() {
    assert_eq!(
        convert_windows_path_to_wsl(r"C:\Users\aloke\file.txt"),
        "/mnt/c/Users/aloke/file.txt"
    );
    assert_eq!(
        convert_windows_path_to_wsl(r"D:\Pictures\Screenshots\Screenshot 2025-05-14 155816.png"),
        "/mnt/d/Pictures/Screenshots/Screenshot 2025-05-14 155816.png"
    );
    // Drive letter only
    assert_eq!(convert_windows_path_to_wsl(r"C:\"), "/mnt/c");
    assert_eq!(convert_windows_path_to_wsl("C:"), "/mnt/c");
    // Uppercase drive letter should be lowercased
    assert_eq!(convert_windows_path_to_wsl(r"E:\foo"), "/mnt/e/foo");
    // Non-drive path (e.g. UNC) gets backslashes replaced
    assert_eq!(
        convert_windows_path_to_wsl(r"\\server\share\file"),
        "//server/share/file"
    );
}

#[test]
fn test_convert_windows_path_to_msys2() {
    assert_eq!(
        convert_windows_path_to_msys2(r"C:\Users\aloke\file.txt"),
        "/c/Users/aloke/file.txt"
    );
    assert_eq!(
        convert_windows_path_to_msys2(r"D:\Pictures\Screenshots\Screenshot 2025-05-14 155816.png"),
        "/d/Pictures/Screenshots/Screenshot 2025-05-14 155816.png"
    );
    // Drive letter only
    assert_eq!(convert_windows_path_to_msys2(r"C:\"), "/c");
    assert_eq!(convert_windows_path_to_msys2("C:"), "/c");
    // Uppercase drive letter should be lowercased
    assert_eq!(convert_windows_path_to_msys2(r"E:\foo"), "/e/foo");
    // Non-drive path (e.g. UNC) gets backslashes replaced
    assert_eq!(
        convert_windows_path_to_msys2(r"\\server\share\file"),
        "//server/share/file"
    );
}

#[test]
fn test_canonicalize_git_bash_path() {
    assert_eq!(
        canonicalize_git_bash_path(
            Path::new("C:")
                .join("Program Files")
                .join("Git")
                .join("bin")
                .join("bash.exe")
        ),
        Path::new("C:")
            .join("Program Files")
            .join("Git")
            .join("usr")
            .join("bin")
            .join("bash.exe")
    );
    assert_eq!(
        canonicalize_git_bash_path(
            Path::new("C:")
                .join("Windows")
                .join("system32")
                .join("bash.exe")
        ),
        Path::new("C:")
            .join("Windows")
            .join("system32")
            .join("bash.exe")
    );
}
