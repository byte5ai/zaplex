use repo_metadata::{
    entry::{DirectoryEntry, Entry, FileMetadata},
    file_tree_store::FileTreeState,
    repositories::DetectedRepositories,
    DirectoryWatcher, RepoMetadataModel,
};
use virtual_fs::{Stub, VirtualFS};
use warpui::App;

use super::{find_skill_directories_in_tree, read_skills_from_directories};

// ============================================================================
// Tests for find_skill_directories_in_tree
// ============================================================================

#[test]
fn find_skill_directories_in_tree_finds_root_skills() {
    VirtualFS::test("find_root_skills", |dirs, mut vfs| {
        let repo = dirs.tests().join("repo");

        vfs.mkdir("repo/.agents/skills/root-skill-1")
            .mkdir("repo/.claude/skills/root-skill-2")
            .with_files(vec![
                Stub::FileWithContent(
                    "repo/.agents/skills/root-skill-1/SKILL.md",
                    "---\nname: root-skill-1\ndescription: test\n---\n# root-skill-1",
                ),
                Stub::FileWithContent(
                    "repo/.claude/skills/root-skill-2/SKILL.md",
                    "---\nname: root-skill-2\ndescription: test\n---\n# root-skill-2",
                ),
            ]);

        let skill1_file = Entry::File(FileMetadata::new(
            repo.join(".agents/skills/root-skill-1/SKILL.md"),
            false,
        ));
        let skill1_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents/skills/root-skill-1"),
            )
            .unwrap(),
            children: vec![skill1_file],
            ignored: false,
            loaded: true,
        });
        let warp_skills = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents/skills"),
            )
            .unwrap(),
            children: vec![skill1_dir],
            ignored: false,
            loaded: true,
        });
        let warp_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents"),
            )
            .unwrap(),
            children: vec![warp_skills],
            ignored: false,
            loaded: true,
        });

        let skill2_file = Entry::File(FileMetadata::new(
            repo.join(".claude/skills/root-skill-2/SKILL.md"),
            false,
        ));
        let skill2_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".claude/skills/root-skill-2"),
            )
            .unwrap(),
            children: vec![skill2_file],
            ignored: false,
            loaded: true,
        });
        let claude_skills = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".claude/skills"),
            )
            .unwrap(),
            children: vec![skill2_dir],
            ignored: false,
            loaded: true,
        });
        let claude_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".claude"),
            )
            .unwrap(),
            children: vec![claude_skills],
            ignored: false,
            loaded: true,
        });

        let root = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(&repo).unwrap(),
            children: vec![warp_dir, claude_dir],
            ignored: false,
            loaded: true,
        });

        App::test((), |mut app| async move {
            let watcher = app.add_singleton_model(DirectoryWatcher::new);
            app.add_singleton_model(|_| DetectedRepositories::default());
            let repo_handle = watcher.update(&mut app, |w, ctx| {
                w.add_directory(
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap(),
                    ctx,
                )
                .unwrap()
            });
            let state = FileTreeState::new(root, vec![], Some(repo_handle));

            let model_handle = app.add_singleton_model(RepoMetadataModel::new);
            model_handle.update(&mut app, |model, ctx| {
                let key =
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap();
                model.insert_test_state(key, state, ctx);
            });

            model_handle.read(&app, |model, ctx| {
                let skill_dirs = find_skill_directories_in_tree(&repo, model, ctx);
                assert_eq!(skill_dirs.len(), 2);
                assert!(skill_dirs.contains(&repo.join(".agents/skills")));
                assert!(skill_dirs.contains(&repo.join(".claude/skills")));

                let skills = read_skills_from_directories(skill_dirs);
                assert_eq!(skills.len(), 2);
                let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
                assert!(names.contains(&"root-skill-1"));
                assert!(names.contains(&"root-skill-2"));
            });
        });
    });
}

#[test]
fn find_skill_directories_in_tree_finds_subdirectory_skills() {
    VirtualFS::test("find_subdir_skills", |dirs, mut vfs| {
        let repo = dirs.tests().join("repo");

        vfs.mkdir("repo/.agents/skills/root-skill")
            .mkdir("repo/packages/frontend/.agents/skills/frontend-skill")
            .with_files(vec![
                Stub::FileWithContent(
                    "repo/.agents/skills/root-skill/SKILL.md",
                    "---\nname: root-skill\ndescription: test\n---\n# root-skill",
                ),
                Stub::FileWithContent(
                    "repo/packages/frontend/.agents/skills/frontend-skill/SKILL.md",
                    "---\nname: frontend-skill\ndescription: test\n---\n# frontend-skill",
                ),
            ]);

        let root_skill_file = Entry::File(FileMetadata::new(
            repo.join(".agents/skills/root-skill/SKILL.md"),
            false,
        ));
        let root_skill_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents/skills/root-skill"),
            )
            .unwrap(),
            children: vec![root_skill_file],
            ignored: false,
            loaded: true,
        });
        let root_warp_skills = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents/skills"),
            )
            .unwrap(),
            children: vec![root_skill_dir],
            ignored: false,
            loaded: true,
        });
        let root_warp = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents"),
            )
            .unwrap(),
            children: vec![root_warp_skills],
            ignored: false,
            loaded: true,
        });

        let frontend_skill_file = Entry::File(FileMetadata::new(
            repo.join("packages/frontend/.agents/skills/frontend-skill/SKILL.md"),
            false,
        ));
        let frontend_skill_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages/frontend/.agents/skills/frontend-skill"),
            )
            .unwrap(),
            children: vec![frontend_skill_file],
            ignored: false,
            loaded: true,
        });
        let frontend_warp_skills = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages/frontend/.agents/skills"),
            )
            .unwrap(),
            children: vec![frontend_skill_dir],
            ignored: false,
            loaded: true,
        });
        let frontend_warp = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages/frontend/.agents"),
            )
            .unwrap(),
            children: vec![frontend_warp_skills],
            ignored: false,
            loaded: true,
        });
        let frontend = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages/frontend"),
            )
            .unwrap(),
            children: vec![frontend_warp],
            ignored: false,
            loaded: true,
        });
        let packages = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages"),
            )
            .unwrap(),
            children: vec![frontend],
            ignored: false,
            loaded: true,
        });

        let root = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(&repo).unwrap(),
            children: vec![root_warp, packages],
            ignored: false,
            loaded: true,
        });

        App::test((), |mut app| async move {
            let watcher = app.add_singleton_model(DirectoryWatcher::new);
            app.add_singleton_model(|_| DetectedRepositories::default());
            let repo_handle = watcher.update(&mut app, |w, ctx| {
                w.add_directory(
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap(),
                    ctx,
                )
                .unwrap()
            });
            let state = FileTreeState::new(root, vec![], Some(repo_handle));

            let model_handle = app.add_singleton_model(RepoMetadataModel::new);
            model_handle.update(&mut app, |model, ctx| {
                let key =
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap();
                model.insert_test_state(key, state, ctx);
            });

            model_handle.read(&app, |model, ctx| {
                let skill_dirs = find_skill_directories_in_tree(&repo, model, ctx);
                assert_eq!(skill_dirs.len(), 2);
                assert!(skill_dirs.contains(&repo.join(".agents/skills")));
                assert!(skill_dirs.contains(&repo.join("packages/frontend/.agents/skills")));

                let skills = read_skills_from_directories(skill_dirs);
                assert_eq!(skills.len(), 2);
                let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
                assert!(names.contains(&"root-skill"));
                assert!(names.contains(&"frontend-skill"));
            });
        });
    });
}

#[test]
fn find_skill_directories_in_tree_empty_repo() {
    VirtualFS::test("find_skills_empty", |dirs, mut vfs| {
        let repo = dirs.tests().join("repo");
        vfs.mkdir("repo/src");

        let src = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(&repo.join("src"))
                .unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        });
        let root = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(&repo).unwrap(),
            children: vec![src],
            ignored: false,
            loaded: true,
        });

        App::test((), |mut app| async move {
            let watcher = app.add_singleton_model(DirectoryWatcher::new);
            app.add_singleton_model(|_| DetectedRepositories::default());
            let repo_handle = watcher.update(&mut app, |w, ctx| {
                w.add_directory(
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap(),
                    ctx,
                )
                .unwrap()
            });
            let state = FileTreeState::new(root, vec![], Some(repo_handle));

            let model_handle = app.add_singleton_model(RepoMetadataModel::new);
            model_handle.update(&mut app, |model, ctx| {
                let key =
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap();
                model.insert_test_state(key, state, ctx);
            });

            model_handle.read(&app, |model, ctx| {
                let skill_dirs = find_skill_directories_in_tree(&repo, model, ctx);
                assert!(skill_dirs.is_empty());
            });
        });
    });
}
