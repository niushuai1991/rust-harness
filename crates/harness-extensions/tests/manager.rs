use harness_extensions::PluginManager;

#[test]
fn plugin_command_hooks_require_explicit_manifest_permission() {
    let data = tempfile::tempdir().unwrap();
    let package = tempfile::tempdir().unwrap();
    std::fs::write(
        package.path().join("plugin.toml"),
        "id='command-hook'\nname='Command Hook'\nversion='1.0.0'\napi='^0.1'\n",
    )
    .unwrap();
    std::fs::create_dir(package.path().join("hooks")).unwrap();
    std::fs::write(
        package.path().join("hooks/hooks.json"),
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"example"}]}]}}"#,
    )
    .unwrap();
    let manager = PluginManager::new(data.path().to_path_buf()).unwrap();
    assert!(
        manager.install(package.path(), vec![]).is_err(),
        "插件命令 Hook 必须显式声明权限"
    );
    assert!(manager.list().unwrap().is_empty());
}

#[test]
fn plugin_command_hooks_cannot_be_enabled_without_explicit_grant() {
    let data = tempfile::tempdir().unwrap();
    let package = tempfile::tempdir().unwrap();
    std::fs::write(package.path().join("plugin.toml"), "id='command-hook'\nname='Command Hook'\nversion='1.0.0'\napi='^0.1'\npermissions=['hooks.command']\n").unwrap();
    std::fs::create_dir(package.path().join("hooks")).unwrap();
    std::fs::write(
        package.path().join("hooks/hooks.json"),
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"example"}]}]}}"#,
    )
    .unwrap();
    let manager = PluginManager::new(data.path().to_path_buf()).unwrap();
    manager.install(package.path(), vec![]).unwrap();
    assert!(
        manager.set_enabled("command-hook", true).is_err(),
        "未授予权限不能启用命令 Hook"
    );
}

#[test]
fn plugin_command_hook_sources_require_enabled_plugin_grant_and_workspace_policy() {
    let data = tempfile::tempdir().unwrap();
    let package = tempfile::tempdir().unwrap();
    std::fs::write(package.path().join("plugin.toml"), "id='command-hook'\nname='Command Hook'\nversion='1.0.0'\napi='^0.1'\npermissions=['hooks.command']\n").unwrap();
    std::fs::create_dir(package.path().join("hooks")).unwrap();
    std::fs::write(
        package.path().join("hooks/hooks.json"),
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"example"}]}]}}"#,
    )
    .unwrap();
    let manager = PluginManager::new(data.path().to_path_buf()).unwrap();
    manager
        .install(package.path(), vec!["hooks.command".into()])
        .unwrap();
    manager.set_workspace_policy("workspace", vec!["hooks.command".into()]);
    assert!(
        manager
            .command_hook_sources("workspace")
            .unwrap()
            .is_empty()
    );
    manager.set_enabled("command-hook", true).unwrap();
    assert_eq!(manager.command_hook_sources("workspace").unwrap().len(), 1);
    assert!(manager.command_hook_sources("other").unwrap().is_empty());
    manager.set_enabled("command-hook", false).unwrap();
    assert!(
        manager
            .command_hook_sources("workspace")
            .unwrap()
            .is_empty()
    );
}
use std::io::Write;
#[test]
fn command_hook_package_changes_invalidate_the_grant() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("hooks")).unwrap();
    std::fs::write(src.path().join("plugin.toml"), "id='secure-hook'\nname='Secure'\nversion='1.0.0'\napi='^0.1'\npermissions=['hooks.command']").unwrap();
    std::fs::write(src.path().join("hooks/hooks.json"), "{}").unwrap();
    std::fs::write(src.path().join("script.ps1"), "exit 0").unwrap();
    let manager = PluginManager::new(data.path().into()).unwrap();
    manager
        .install(src.path(), vec!["hooks.command".into()])
        .unwrap();
    manager.set_enabled("secure-hook", true).unwrap();
    manager.set_workspace_policy("workspace", vec!["hooks.command".into()]);
    assert_eq!(manager.command_hook_sources("workspace").unwrap().len(), 1);
    std::fs::write(
        data.path().join("versions/secure-hook/1.0.0/script.ps1"),
        "exit 2",
    )
    .unwrap();
    assert!(manager.command_hook_sources("workspace").is_err());
}

#[test]
fn removing_plugin_hook_config_cannot_silently_disable_it() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("hooks")).unwrap();
    std::fs::write(src.path().join("plugin.toml"), "id='secure-hook'\nname='Secure'\nversion='1.0.0'\napi='^0.1'\npermissions=['hooks.command']").unwrap();
    std::fs::write(src.path().join("hooks/hooks.json"), "{}").unwrap();
    let manager = PluginManager::new(data.path().into()).unwrap();
    manager
        .install(src.path(), vec!["hooks.command".into()])
        .unwrap();
    manager.set_enabled("secure-hook", true).unwrap();
    manager.set_workspace_policy("workspace", vec!["hooks.command".into()]);
    std::fs::remove_file(
        data.path()
            .join("versions/secure-hook/1.0.0/hooks/hooks.json"),
    )
    .unwrap();
    assert!(manager.command_hook_sources("workspace").is_err());
}
#[test]
fn failed_install_does_not_publish() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::write(
        src.path().join("plugin.toml"),
        "id='bad'\nname='bad'\nversion='1.0.0'\napi='^9'\n",
    )
    .unwrap();
    let manager = PluginManager::new(data.path().to_owned()).unwrap();
    assert!(manager.install(src.path(), vec![]).is_err());
    assert!(manager.list().unwrap().is_empty());
}
#[test]
fn rejects_archive_traversal_ads_and_case_duplicate() {
    for entries in [
        vec!["../escape"],
        vec!["C:/x"],
        vec!["x:stream"],
        vec!["//server/file"],
        vec!["a", "A"],
        vec!["NUL.txt"],
    ] {
        let data = tempfile::tempdir().unwrap();
        let archive = data.path().join("attack.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        for name in entries {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"x").unwrap();
        }
        zip.finish().unwrap();
        let m = PluginManager::new(data.path().join("plugins")).unwrap();
        assert!(m.install(&archive, vec![]).is_err());
        assert!(m.list().unwrap().is_empty());
    }
}
#[test]
fn version_is_immutable_and_install_never_auto_grants() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::write(
        src.path().join("plugin.toml"),
        "id='theme'\nname='Theme'\nversion='1.0.0'\napi='^0.1'\npermissions=['log']",
    )
    .unwrap();
    let m = PluginManager::new(data.path().into()).unwrap();
    let p = m.install(src.path(), vec![]).unwrap();
    assert!(!p.enabled);
    assert!(p.permissions.is_empty());
    assert!(m.install(src.path(), vec![]).is_err());
    assert!(m.install(src.path(), vec!["http".into()]).is_err());
    m.set_enabled("theme", true).unwrap();
    drop(m);
    let m = PluginManager::new(data.path().into()).unwrap();
    assert!(m.list().unwrap()[0].enabled);
    m.remove("theme").unwrap();
    assert!(m.list().unwrap().is_empty());
}
#[test]
fn rejects_zip_symlinks_and_compression_bombs() {
    for bomb in [false, true] {
        let data = tempfile::tempdir().unwrap();
        let archive = data.path().join("attack.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        if bomb {
            zip.start_file(
                "bomb",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            zip.write_all(&vec![0; 2 << 20]).unwrap();
        } else {
            zip.add_symlink(
                "link",
                "../outside",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        }
        zip.finish().unwrap();
        let m = PluginManager::new(data.path().join("plugins")).unwrap();
        assert!(m.install(&archive, vec![]).is_err());
        assert!(m.list().unwrap().is_empty());
    }
}
#[test]
fn dependencies_and_api_failure_preserve_installed_registry() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let m = PluginManager::new(data.path().into()).unwrap();
    std::fs::write(src.path().join("plugin.toml"),"id='dependent'\nname='Dependent'\nversion='1.0.0'\napi='^0.1'\n[dependencies]\nmissing='^1'").unwrap();
    assert!(m.install(src.path(), vec![]).is_err());
    assert!(m.list().unwrap().is_empty());
}
#[test]
fn upgrading_enabled_dependency_is_rejected_without_registry_changes() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let m = PluginManager::new(data.path().into()).unwrap();
    let write = |id: &str, version: &str, deps: &str| {
        std::fs::write(
            src.path().join("plugin.toml"),
            format!("id='{id}'\nname='{id}'\nversion='{version}'\napi='^0.1'\n{deps}"),
        )
        .unwrap()
    };
    write("base", "1.0.0", "");
    m.install(src.path(), vec![]).unwrap();
    m.set_enabled("base", true).unwrap();
    write("dependent", "1.0.0", "[dependencies]\nbase='^1'");
    m.install(src.path(), vec![]).unwrap();
    m.set_enabled("dependent", true).unwrap();
    write("base", "1.1.0", "");
    assert!(m.install(src.path(), vec![]).is_err());
    let plugins = m.list().unwrap();
    assert!(plugins.iter().all(|p| p.enabled));
    assert_eq!(
        plugins.iter().find(|p| p.id == "base").unwrap().version,
        "1.0.0"
    );
    assert!(data.path().join("versions/base/1.0.0/plugin.toml").exists());
    assert!(!data.path().join("versions/base/1.1.0").exists());
}
