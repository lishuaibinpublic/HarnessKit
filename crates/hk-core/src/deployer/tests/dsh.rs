//! dsh managed-block tests (moved from the flat deployer.rs).

use super::*;

#[cfg(test)]
mod dsh_toggle_tests {
    use super::*;

    const HOME_WITH_GH: &str = r#"# precious comment
- insert:
    - id: mcp-github
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: github
        transport: stdio
        command: npx
        env:
          GITHUB_TOKEN: !!js process.env.GITHUB_TOKEN
"#;

    fn patch_file(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cordis.patch.yml");
        std::fs::write(&path, text).unwrap();
        (tmp, path)
    }

    #[test]
    fn disable_appends_managed_block_and_enable_removes_it() {
        let (_tmp, path) = patch_file(HOME_WITH_GH);

        set_dsh_mcp_enabled(&path, "github", false).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(HOME_WITH_GH), "user bytes preserved verbatim");
        assert!(text.contains("- id: mcp-github\n  disabled: true"));
        assert!(text.contains("managed by HarnessKit"));

        // Enable: base state (the insert) is already enabled → block entry
        // removed entirely; user content restored byte-for-byte.
        set_dsh_mcp_enabled(&path, "github", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), HOME_WITH_GH);
    }

    #[test]
    fn enable_overrides_user_disable_with_disabled_false() {
        // User disabled it themselves → HK writes an explicit disabled: false
        // override (upstream e2e-covered semantics).
        let text = format!("{HOME_WITH_GH}- id: mcp-github\n  disabled: true\n");
        let (_tmp, path) = patch_file(&text);
        set_dsh_mcp_enabled(&path, "github", true).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.starts_with(&text), "user bytes preserved");
        assert!(out.contains("- id: mcp-github\n  disabled: false"));
    }

    #[test]
    fn template_file_toggle_errors_not_found() {
        // dsh's seeded patch template: comment header + literal []. No row
        // exists in it → toggling must error, and the file must be untouched.
        let template = "# header comment\n[]\n";
        let (_tmp, path) = patch_file(template);
        let err = set_dsh_mcp_enabled(&path, "github", false).unwrap_err();
        assert!(matches!(err, HkError::NotFound(_)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), template);
    }

    #[test]
    fn roundtrip_always_leaves_valid_yaml_list() {
        // After any disable→enable cycle the file must re-parse as a YAML
        // list — an empty/comment-only patch file is a dsh boot error.
        let (_tmp, path) = patch_file(HOME_WITH_GH);
        set_dsh_mcp_enabled(&path, "github", false).unwrap();
        set_dsh_mcp_enabled(&path, "github", true).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert!(parsed.is_sequence(), "file must stay a valid YAML list");
    }

    #[test]
    fn unknown_server_errors() {
        let (_tmp, path) = patch_file(HOME_WITH_GH);
        let err = set_dsh_mcp_enabled(&path, "nope", false).unwrap_err();
        assert!(matches!(err, HkError::NotFound(_)));
    }

    #[test]
    fn toggle_is_idempotent() {
        let (_tmp, path) = patch_file(HOME_WITH_GH);
        set_dsh_mcp_enabled(&path, "github", false).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();
        set_dsh_mcp_enabled(&path, "github", false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
    }

    #[test]
    fn unbalanced_markers_error_and_leave_file_untouched() {
        let text = format!(
            "{HOME_WITH_GH}{}\n- id: mcp-github\n  disabled: true\n",
            DSH_BLOCK_BEGIN
        );
        let (_tmp, path) = patch_file(&text); // BEGIN without END
        let err = set_dsh_mcp_enabled(&path, "github", false).unwrap_err();
        assert!(matches!(err, HkError::ConfigCorrupted(_)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn malformed_block_content_errors_and_leaves_the_file_untouched() {
        // HK owns every byte inside the markers, so anything it could not have
        // rendered is corruption: refuse to write rather than re-render the
        // block without it. Each case is a DIFFERENT way that can happen.
        for (case, body) in [
            // Unparseable YAML.
            ("unclosed flow sequence", "- id: [unclosed\n"),
            // Parses, but HK never renders an entry shaped like this — a
            // silent drop here would delete whatever the user meant by it.
            ("entry HK never renders", "- surprise: true\n"),
            // Extra keys on an id-targeted entry are LIVE dsh patch semantics
            // (they patch the target row), so dropping them on re-render would
            // alter the user's effective config.
            (
                "extra keys on a toggle entry",
                "- id: mcp-github\n  disabled: true\n  command: pwned\n",
            ),
            (
                "extra keys beside an insert group",
                "- insert:\n    - id: mcp-x\n  after: mcp-github\n",
            ),
        ] {
            let text = format!("{HOME_WITH_GH}{DSH_BLOCK_BEGIN}\n{body}{DSH_BLOCK_END}\n");
            let (_tmp, path) = patch_file(&text);
            let err = set_dsh_mcp_enabled(&path, "github", false).unwrap_err();
            assert!(matches!(err, HkError::ConfigCorrupted(_)), "{case}: {err:?}");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                text,
                "{case}: file must be untouched"
            );
        }
    }

    #[test]
    fn hk_inserted_server_toggles_via_own_row_and_render_is_byte_stable() {
        // Pins the serde_yaml insert byte format BEFORE T8 depends on it.
        let text = format!(
            "{HOME_WITH_GH}{DSH_BLOCK_BEGIN}\n- insert:\n    - id: mcp-web\n      name: '@deepseek-ai/dsh-mcp-client'\n      config:\n        serverName: web\n        transport: streamable-http\n        url: http://localhost:3000/mcp\n{DSH_BLOCK_END}\n"
        );
        let (_tmp, path) = patch_file(&text);
        set_dsh_mcp_enabled(&path, "web", false).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.starts_with(HOME_WITH_GH), "user bytes preserved");

        // Disable lands on the insert row's OWN `disabled` field — never a
        // separate toggle entry.
        let (user_text, block) = split_dsh_managed_block(&out).unwrap();
        assert!(block.toggles.is_empty(), "no separate toggle entry");
        assert_eq!(block.inserts.len(), 1);
        assert_eq!(
            block.inserts[0].get("disabled").and_then(|v| v.as_bool()),
            Some(true)
        );

        let parsed: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert!(parsed.is_sequence(), "file must stay a valid YAML list");

        // Byte-stable: a second split→render reproduces the file exactly.
        assert_eq!(render_dsh_patch(&user_text, &block), out);
    }

    #[test]
    fn user_content_after_block_survives_roundtrip() {
        // Documented out-vote mechanism: user lines AFTER the managed block
        // must never be lost. (They may legitimately be reordered before the
        // re-appended block on the next toggle — base-state semantics.)
        let (_tmp, path) = patch_file(HOME_WITH_GH);
        set_dsh_mcp_enabled(&path, "github", false).unwrap();
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("# user note after block\n");
        std::fs::write(&path, &text).unwrap();
        set_dsh_mcp_enabled(&path, "github", true).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# user note after block"));
        assert!(!out.contains("managed by HarnessKit"), "override no longer needed");
    }

    #[test]
    fn two_servers_share_one_managed_block() {
        let text = format!(
            "{HOME_WITH_GH}    - id: mcp-web\n      name: '@deepseek-ai/dsh-mcp-client'\n      config:\n        serverName: web\n        transport: streamable-http\n        url: http://localhost:3000/mcp\n"
        );
        let (_tmp, path) = patch_file(&text);
        set_dsh_mcp_enabled(&path, "github", false).unwrap();
        set_dsh_mcp_enabled(&path, "web", false).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert_eq!(out.matches(DSH_BLOCK_BEGIN).count(), 1, "exactly one block");
        assert!(out.contains("- id: mcp-github\n  disabled: true"));
        assert!(out.contains("- id: mcp-web\n  disabled: true"));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert!(parsed.is_sequence());
    }
}

#[cfg(test)]
mod dsh_plugin_toggle_tests {
    use super::*;

    const PROFILE_PATCH: &str =
        "- insert:\n    - id: tool-policy\n      name: dsh-plugin-tool\n      config:\n        mode: strict\n";

    fn dsh_home_with_profile(
        patch: &str,
        home_patch: Option<&str>,
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dsh_home = tmp.path().join(".dsh");
        std::fs::create_dir_all(dsh_home.join("profiles/web")).unwrap();
        std::fs::write(dsh_home.join("profiles/web/cordis.patch.yml"), patch).unwrap();
        if let Some(h) = home_patch {
            std::fs::write(dsh_home.join("cordis.patch.yml"), h).unwrap();
        }
        (tmp, dsh_home)
    }

    /// The layer that DEFINES the row in the `dsh_home_with_profile` fixture
    /// — what the dsh adapter puts on the entry the UI toggled.
    fn web_layer(dsh_home: &Path) -> std::path::PathBuf {
        dsh_home.join("profiles/web/cordis.patch.yml")
    }

    /// The home patch the writer edits — what `manager::toggle_plugin` passes
    /// straight through from the dsh adapter's `mcp_config_path()`.
    fn home_patch(dsh_home: &Path) -> std::path::PathBuf {
        dsh_home.join("cordis.patch.yml")
    }

    #[test]
    fn disable_profile_row_writes_home_block_and_enable_removes_it() {
        let (_tmp, dsh_home) = dsh_home_with_profile(PROFILE_PATCH, None);
        set_dsh_plugin_enabled(
            &home_patch(&dsh_home),
            "tool-policy",
            false,
            &[web_layer(&dsh_home)],
        )
        .unwrap();
        let home = std::fs::read_to_string(dsh_home.join("cordis.patch.yml")).unwrap();
        assert!(home.contains("managed by HarnessKit"));
        assert!(home.contains("- id: tool-policy\n  disabled: true"));
        // ONLY the home file is written — profile patch stays byte-identical.
        assert_eq!(
            std::fs::read_to_string(dsh_home.join("profiles/web/cordis.patch.yml")).unwrap(),
            PROFILE_PATCH
        );
        set_dsh_plugin_enabled(
            &home_patch(&dsh_home),
            "tool-policy",
            true,
            &[web_layer(&dsh_home)],
        )
        .unwrap();
        let home = std::fs::read_to_string(dsh_home.join("cordis.patch.yml")).unwrap();
        assert!(!home.contains("managed by HarnessKit"), "back to base → entry removed");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&home).unwrap();
        assert!(parsed.is_sequence(), "file must stay a valid YAML list");
    }

    #[test]
    fn enable_user_disabled_row_writes_disabled_false_override() {
        // The row is disabled IN THE PROFILE FILE by the user; HK enable must
        // write an explicit `disabled: false` override (last layer wins).
        let patch = format!("{PROFILE_PATCH}- id: tool-policy\n  disabled: true\n");
        let (_tmp, dsh_home) = dsh_home_with_profile(&patch, None);
        set_dsh_plugin_enabled(
            &home_patch(&dsh_home),
            "tool-policy",
            true,
            &[web_layer(&dsh_home)],
        )
        .unwrap();
        let home = std::fs::read_to_string(dsh_home.join("cordis.patch.yml")).unwrap();
        assert!(home.contains("- id: tool-policy\n  disabled: false"));
    }

    #[test]
    fn home_user_override_is_part_of_base_state() {
        // User already disabled the row from their home patch text: HK
        // disable is then a no-op (no block written).
        let (_tmp, dsh_home) = dsh_home_with_profile(
            PROFILE_PATCH,
            Some("- id: tool-policy\n  disabled: true\n"),
        );
        set_dsh_plugin_enabled(
            &home_patch(&dsh_home),
            "tool-policy",
            false,
            &[web_layer(&dsh_home)],
        )
        .unwrap();
        let home = std::fs::read_to_string(dsh_home.join("cordis.patch.yml")).unwrap();
        assert!(!home.contains("managed by HarnessKit"), "already disabled at base");
    }

    #[test]
    fn unknown_row_errors_not_found_and_writes_nothing() {
        let (_tmp, dsh_home) = dsh_home_with_profile(PROFILE_PATCH, None);
        let err =
            set_dsh_plugin_enabled(
                &home_patch(&dsh_home),
                "nope",
                false,
                &[web_layer(&dsh_home)],
            )
            .unwrap_err();
        assert!(matches!(err, HkError::NotFound(_)));
        assert!(!dsh_home.join("cordis.patch.yml").exists(), "nothing written");
    }

    #[test]
    fn sibling_profile_override_is_not_part_of_base_state() {
        // Row DEFINED (enabled) in profile `alpha`, separately overridden
        // `disabled: true` in profile `beta`. dsh boots ONE profile at a
        // time — beta's patch is never loaded next to alpha's — so from
        // alpha's entry the base state is ENABLED and disabling it must
        // actually write an override. Folding beta in would compute
        // base=disabled and silently write nothing, leaving the plugin
        // loaded in alpha.
        let tmp = tempfile::tempdir().unwrap();
        let dsh_home = tmp.path().join(".dsh");
        std::fs::create_dir_all(dsh_home.join("profiles/alpha")).unwrap();
        std::fs::create_dir_all(dsh_home.join("profiles/beta")).unwrap();
        std::fs::write(dsh_home.join("profiles/alpha/cordis.patch.yml"), PROFILE_PATCH).unwrap();
        std::fs::write(
            dsh_home.join("profiles/beta/cordis.patch.yml"),
            "- id: tool-policy\n  disabled: true\n",
        )
        .unwrap();
        let alpha_layer = dsh_home.join("profiles/alpha/cordis.patch.yml");

        set_dsh_plugin_enabled(&home_patch(&dsh_home), "tool-policy", false, std::slice::from_ref(&alpha_layer)).unwrap();
        let home = std::fs::read_to_string(dsh_home.join("cordis.patch.yml")).unwrap();
        assert!(
            home.contains("- id: tool-policy\n  disabled: true"),
            "alpha's row is enabled at base, so disable must write: {home}"
        );

        // And back: the row returns to alpha's own base → block removed. A
        // fold that consulted beta would instead leave a gratuitous
        // `disabled: false`, overriding beta's own choice machine-wide.
        set_dsh_plugin_enabled(&home_patch(&dsh_home), "tool-policy", true, std::slice::from_ref(&alpha_layer)).unwrap();
        let home = std::fs::read_to_string(dsh_home.join("cordis.patch.yml")).unwrap();
        assert!(
            !home.contains("managed by HarnessKit"),
            "back to alpha's base → entry removed: {home}"
        );
        assert!(!home.contains("disabled: false"), "no gratuitous override: {home}");
    }

    #[test]
    fn own_layer_override_after_the_definition_is_part_of_base_state() {
        // Same row defined AND overridden inside the toggled entry's own
        // layer: that override is loaded with the definition, so it does
        // count — the per-profile rule narrows the fold, it does not drop
        // in-layer ordering.
        let patch = format!("{PROFILE_PATCH}- id: tool-policy\n  disabled: true\n");
        let (_tmp, dsh_home) = dsh_home_with_profile(&patch, None);
        set_dsh_plugin_enabled(
            &home_patch(&dsh_home),
            "tool-policy",
            false,
            &[web_layer(&dsh_home)],
        )
        .unwrap();
        let home = std::fs::read_to_string(dsh_home.join("cordis.patch.yml"))
            .unwrap_or_else(|_| String::new());
        assert!(
            !home.contains("managed by HarnessKit"),
            "the row's own layer already disables it at base: {home}"
        );
    }

    #[test]
    fn corrupted_block_error_names_the_home_patch_path() {
        // The call-site map_err must append the file path + remediation hint
        // to the path-agnostic block parser's ConfigCorrupted.
        let bad_home = format!("{DSH_BLOCK_BEGIN}\n- surprise: 1\n{DSH_BLOCK_END}\n");
        let (_tmp, dsh_home) = dsh_home_with_profile(PROFILE_PATCH, Some(&bad_home));
        let err = set_dsh_plugin_enabled(
            &home_patch(&dsh_home),
            "tool-policy",
            false,
            &[web_layer(&dsh_home)],
        )
            .unwrap_err();
        let HkError::ConfigCorrupted(msg) = err else {
            panic!("expected ConfigCorrupted, got {err:?}");
        };
        let home_path = dsh_home.join("cordis.patch.yml").display().to_string();
        assert!(msg.contains(&home_path), "message names the file: {msg}");
        assert!(msg.contains("fix or remove"), "message carries the hint: {msg}");
        // Nothing written: the corrupt file is untouched.
        assert_eq!(
            std::fs::read_to_string(dsh_home.join("cordis.patch.yml")).unwrap(),
            bad_home
        );
    }

    #[test]
    fn home_defined_row_toggles_too() {
        // Rows defined directly in the home layer are also valid targets:
        // the owning layer IS the home patch, and the writer must then read
        // that layer only through the block-stripped user text.
        let (_tmp, dsh_home) = dsh_home_with_profile(
            "[]\n",
            Some("- insert:\n    - id: theme-row\n      name: dsh-plugin-theme\n"),
        );
        let home_layer = dsh_home.join("cordis.patch.yml");
        set_dsh_plugin_enabled(&home_patch(&dsh_home), "theme-row", false, std::slice::from_ref(&home_layer)).unwrap();
        let home = std::fs::read_to_string(dsh_home.join("cordis.patch.yml")).unwrap();
        assert!(home.contains("- id: theme-row\n  disabled: true"));
        assert!(home.starts_with("- insert:"), "user bytes preserved");
        // Re-enable with our own block already in the file: the block must
        // never be folded back in as "base", or this would look like a no-op.
        set_dsh_plugin_enabled(&home_patch(&dsh_home), "theme-row", true, std::slice::from_ref(&home_layer)).unwrap();
        let home = std::fs::read_to_string(dsh_home.join("cordis.patch.yml")).unwrap();
        assert!(!home.contains("managed by HarnessKit"), "back to base → entry removed");
    }
}

#[cfg(test)]
mod dsh_insert_writer_tests {
    use super::*;
    use crate::adapter::dsh::DshAdapter;

    const USER_GH: &str = r#"# precious comment
- insert:
    - id: mcp-github
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: github
        transport: stdio
        command: npx
"#;

    fn patch_file(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cordis.patch.yml");
        std::fs::write(&path, text).unwrap();
        (tmp, path)
    }

    fn stdio_entry(name: &str) -> McpServerEntry {
        McpServerEntry {
            name: name.into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
            env: std::collections::HashMap::from([(
                "GITHUB_TOKEN".to_string(),
                "tok".to_string(),
            )]),
            transport: McpTransport::Stdio,
            url: None,
            headers: Default::default(),
            enabled: true,
        }
    }

    #[test]
    fn stdio_install_round_trips_through_the_dsh_reader() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cordis.patch.yml");
        // Missing file: writer starts from the valid empty form.
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("github2")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("managed by HarnessKit"));
        // Markers are YAML comments, so the whole file parses as one
        // document and the P0 reader sees the new row with no extra plumbing.
        assert_eq!(DshAdapter::mcp_enabled_in_text(&text).get("github2"), Some(&true));
        assert_eq!(
            DshAdapter::mcp_row_id_in_text(&text, "github2").as_deref(),
            Some("mcp-github2")
        );
        // Explicit discriminant + serverName always (mcp-client schema).
        assert!(text.contains("transport: stdio"));
        assert!(text.contains("serverName: github2"));
        assert!(text.contains("command: npx"));
        assert!(text.contains("GITHUB_TOKEN: tok"));
    }

    #[test]
    fn install_preserves_user_bytes_and_drops_placeholder() {
        let (_tmp, path) = patch_file("# my notes\n[]\n");
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("github2")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# my notes\n"));
        assert!(
            !text.lines().any(|l| l.trim() == "[]"),
            "placeholder can't coexist with entries"
        );
        let parsed: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        assert!(parsed.is_sequence());
    }

    #[test]
    fn server_name_is_sanitized_to_the_mcp_client_pattern() {
        // /^[A-Za-z0-9_-]{1,32}$/ — invalid chars map to '-', 32-char cap.
        let (_tmp, path) = patch_file("[]\n");
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("My Server/rocks!")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("serverName: My-Server-rocks-"));

        // A name with no valid character at all cannot be sanitized.
        let err = deploy_mcp_server_dsh_cordis(&path, &stdio_entry("///")).unwrap_err();
        assert!(matches!(err, HkError::Validation(_)));
    }

    #[test]
    fn server_name_and_row_id_collisions_error() {
        // serverName collision with a user-authored row.
        let (_tmp, path) = patch_file(USER_GH);
        let err = deploy_mcp_server_dsh_cordis(&path, &stdio_entry("github")).unwrap_err();
        assert!(matches!(&err, HkError::Validation(m) if m.contains("github")));

        // Row-id collision with an unrelated user row occupying the generated id.
        let user2 = "- insert:\n    - id: mcp-github2\n      name: dsh-plugin-tool\n      config:\n        mode: x\n";
        let (_tmp2, path2) = patch_file(user2);
        let err = deploy_mcp_server_dsh_cordis(&path2, &stdio_entry("github2")).unwrap_err();
        assert!(matches!(&err, HkError::Validation(m) if m.contains("mcp-github2")));

        // Double-install of the same HK server collides with its own block row.
        let (_tmp3, path3) = patch_file("[]\n");
        deploy_mcp_server_dsh_cordis(&path3, &stdio_entry("github2")).unwrap();
        let err = deploy_mcp_server_dsh_cordis(&path3, &stdio_entry("github2")).unwrap_err();
        assert!(matches!(err, HkError::Validation(_)));
    }

    #[test]
    fn toggle_of_hk_inserted_server_edits_its_own_insert_entry() {
        let (_tmp, path) = patch_file("[]\n");
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("github2")).unwrap();
        set_dsh_mcp_enabled(&path, "github2", false).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("disabled: true"));
        assert_eq!(
            text.matches("id: mcp-github2").count(),
            1,
            "no separate override row — the insert row itself carries disabled"
        );
        assert_eq!(DshAdapter::mcp_enabled_in_text(&text).get("github2"), Some(&false));

        // ENABLE path of an HK-inserted row: the disabled key is removed from
        // the row itself and the reader sees the server enabled again.
        set_dsh_mcp_enabled(&path, "github2", true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("disabled"), "re-enable removes the disabled key");
        assert_eq!(DshAdapter::mcp_enabled_in_text(&text).get("github2"), Some(&true));
    }

    #[test]
    fn user_row_toggle_back_to_base_keeps_hk_insert_rows() {
        // Spec-pinned: removing a toggle entry must NOT delete co-resident
        // HK insert rows in the same block.
        let (_tmp, path) = patch_file(USER_GH);
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("github2")).unwrap();
        set_dsh_mcp_enabled(&path, "github", false).unwrap();
        set_dsh_mcp_enabled(&path, "github", true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(USER_GH), "user bytes preserved");
        assert!(text.contains("serverName: github2"), "HK insert row survives");
        assert!(
            !text.contains("- id: mcp-github\n  disabled"),
            "toggle entry for the user row is gone"
        );
    }

    #[test]
    fn remove_deletes_hk_row_refuses_user_row_ignores_absent() {
        let (_tmp, path) = patch_file(USER_GH);
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("github2")).unwrap();

        // HK-inserted row: removed; block (now empty) disappears; user bytes intact.
        remove_mcp_server(&path, "github2", McpFormat::DshCordis).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(USER_GH));
        assert!(!text.contains("managed by HarnessKit"));

        // User-authored row: Validation refusal, file untouched.
        let err = remove_mcp_server(&path, "github", McpFormat::DshCordis).unwrap_err();
        assert!(matches!(&err, HkError::Validation(m) if m.contains("cordis.patch.yml")));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        // Absent name: idempotent no-op, like every other format.
        remove_mcp_server(&path, "nope", McpFormat::DshCordis).unwrap();
    }

    #[test]
    fn crlf_user_file_survives_install_byte_for_byte() {
        let user = "# note\r\n- insert:\r\n    - id: mcp-github\r\n      name: '@deepseek-ai/dsh-mcp-client'\r\n      config:\r\n        serverName: github\r\n        transport: stdio\r\n        command: npx\r\n";
        let (_tmp, path) = patch_file(user);
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("web2")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(user), "CRLF user bytes preserved verbatim");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        assert!(parsed.is_sequence());
    }

    #[test]
    fn deploy_mcp_server_dispatch_routes_dsh_cordis() {
        use crate::adapter::AgentAdapter;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".dsh")).unwrap();
        let adapter = DshAdapter::with_home(tmp.path().to_path_buf());
        let path = adapter.mcp_config_path();
        deploy_mcp_server(&path, &stdio_entry("github2"), &adapter).unwrap();
        assert_eq!(adapter.read_mcp_servers().len(), 1);
    }

    /// The bug this pins: installing `microsoft/markitdown` used to write a
    /// row named `microsoft-markitdown` with no record of the original, so
    /// the scanner read back a DIFFERENT extension — the source row's DSH
    /// button never turned ✓, a re-install failed on the serverName
    /// collision, and the list grew a ghost `microsoft-markitdown` row.
    #[test]
    fn install_records_the_original_name_and_the_reader_round_trips_it() {
        use crate::adapter::AgentAdapter;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".dsh")).unwrap();
        let adapter = DshAdapter::with_home(tmp.path().to_path_buf());
        let path = adapter.mcp_config_path();

        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("microsoft/markitdown")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        // On disk: the sanitized name mcp-client accepts, plus the original.
        assert!(text.contains("serverName: microsoft-markitdown"), "{text}");
        assert!(text.contains("_hk_name: microsoft/markitdown"), "{text}");

        // Read back: the ORIGINAL name, so the extension groups with the
        // other agents' rows instead of forming a second one.
        let servers = adapter.read_mcp_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "microsoft/markitdown");

        // Deployer-side lookups still key on the STORED serverName.
        assert_eq!(
            DshAdapter::mcp_row_id_in_text(&text, "microsoft-markitdown").as_deref(),
            Some("mcp-microsoft-markitdown")
        );
        assert!(DshAdapter::mcp_enabled_in_text(&text).contains_key("microsoft-markitdown"));
    }

    #[test]
    fn install_omits_hk_name_when_the_name_needs_no_sanitizing() {
        // Same conditional as Codex: unchanged names keep their exact bytes.
        let (_tmp, path) = patch_file("[]\n");
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("my_server-1")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("serverName: my_server-1"), "{text}");
        assert!(!text.contains("_hk_name"), "{text}");
    }

    #[test]
    fn installing_the_same_original_name_twice_collides_and_adds_no_second_row() {
        use crate::adapter::AgentAdapter;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".dsh")).unwrap();
        let adapter = DshAdapter::with_home(tmp.path().to_path_buf());
        let path = adapter.mcp_config_path();
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("microsoft/markitdown")).unwrap();

        let err =
            deploy_mcp_server_dsh_cordis(&path, &stdio_entry("microsoft/markitdown")).unwrap_err();
        // The message names both the stored name and the original input.
        assert!(matches!(&err, HkError::Validation(m)
            if m.contains("microsoft-markitdown") && m.contains("microsoft/markitdown")));
        assert_eq!(adapter.read_mcp_servers().len(), 1, "no ghost second row");
    }

    #[test]
    fn remove_and_toggle_by_original_name_hit_the_sanitized_row() {
        // Name symmetry: deploy writes the SANITIZED serverName, so remove
        // and toggle called with the ORIGINAL input must normalize the same
        // way — otherwise `remove("My Server")` silently returns Ok while
        // the "My-Server" row stays installed. Asserted on the raw bytes,
        // BELOW the reader, so a reader that also normalized would not hide
        // a writer that did not.
        let (_tmp, path) = patch_file("[]\n");
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("My Server")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("serverName: My-Server"));

        set_dsh_mcp_enabled(&path, "My Server", false).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(DshAdapter::mcp_enabled_in_text(&text).get("My-Server"), Some(&false));

        set_dsh_mcp_enabled(&path, "My Server", true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(DshAdapter::mcp_enabled_in_text(&text).get("My-Server"), Some(&true));

        remove_mcp_server(&path, "My Server", McpFormat::DshCordis).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("My-Server"), "row actually gone: {text}");
    }

    #[test]
    fn build_dsh_insert_row_streamable_http_pins_rendered_bytes() {
        // Byte-level pin of the remote row format, now that dsh advertises
        // RemoteMcpSchema::DshTransport and installs can reach this arm.
        let entry = McpServerEntry {
            name: "web".into(),
            command: String::new(),
            args: vec![],
            env: Default::default(),
            transport: McpTransport::Http,
            url: Some("https://example.com/mcp".into()),
            headers: std::collections::HashMap::from([
                ("X-Api".to_string(), "v1".to_string()),
                ("Authorization".to_string(), "Bearer tok".to_string()),
            ]),
            enabled: true,
        };
        let mut block = DshManagedBlock::default();
        block.inserts.push(build_dsh_insert_row("mcp-web", "web", &entry));
        let out = render_dsh_patch("", &block);
        let expected = format!(
            "{DSH_BLOCK_BEGIN}\n\
             - insert:\n\
             \x20 - id: mcp-web\n\
             \x20   name: '@deepseek-ai/dsh-mcp-client'\n\
             \x20   config:\n\
             \x20     transport: streamable-http\n\
             \x20     serverName: web\n\
             \x20     url: https://example.com/mcp\n\
             \x20     headers:\n\
             \x20       Authorization: Bearer tok\n\
             \x20       X-Api: v1\n\
             {DSH_BLOCK_END}\n"
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn remote_streamable_http_installs_and_sse_is_rejected() {
        use crate::adapter::AgentAdapter;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".dsh")).unwrap();
        let adapter = DshAdapter::with_home(tmp.path().to_path_buf());
        let path = adapter.mcp_config_path();

        let http = McpServerEntry {
            name: "web2".into(),
            command: String::new(),
            args: vec![],
            env: Default::default(),
            transport: McpTransport::Http,
            url: Some("https://example.com/mcp".into()),
            headers: std::collections::HashMap::from([(
                "Authorization".to_string(),
                "Bearer x".to_string(),
            )]),
            enabled: true,
        };
        deploy_mcp_server(&path, &http, &adapter).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("transport: streamable-http"));
        assert!(text.contains("url: https://example.com/mcp"));
        assert!(text.contains("Authorization: Bearer x"));

        let sse = McpServerEntry {
            name: "sse2".into(),
            command: String::new(),
            args: vec![],
            env: Default::default(),
            transport: McpTransport::Sse,
            url: Some("https://example.com/sse".into()),
            headers: Default::default(),
            enabled: true,
        };
        let err = deploy_mcp_server(&path, &sse, &adapter).unwrap_err();
        assert!(matches!(&err, HkError::Validation(m) if m.contains("not SSE")));
        // Rejection happens before any write — the patch file is untouched.
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, text);
    }

    #[test]
    fn distinct_inputs_sanitizing_to_the_same_name_collide() {
        // "My Server" and "My/Server" both sanitize to "My-Server" — the
        // second install must error (collision), never clobber the first,
        // and the message names both the sanitized and the original form.
        let (_tmp, path) = patch_file("[]\n");
        deploy_mcp_server_dsh_cordis(&path, &stdio_entry("My Server")).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let err = deploy_mcp_server_dsh_cordis(&path, &stdio_entry("My/Server")).unwrap_err();
        assert!(
            matches!(&err, HkError::Validation(m)
                if m.contains("'My-Server'") && m.contains("(from 'My/Server')")),
            "got: {err:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "no clobber");
    }

    #[test]
    fn stale_toggle_occupying_the_generated_row_id_errors() {
        // A block toggle can outlive the user row it targeted (dsh warn-skips
        // dangling overrides). Its id still occupies the collision domain: an
        // install deriving the same row id must error, not double-define it.
        let stale = format!("{DSH_BLOCK_BEGIN}\n- id: mcp-x\n  disabled: true\n{DSH_BLOCK_END}\n");
        let (_tmp, path) = patch_file(&stale);
        let err = deploy_mcp_server_dsh_cordis(&path, &stdio_entry("x")).unwrap_err();
        assert!(matches!(&err, HkError::Validation(m) if m.contains("mcp-x")), "got: {err:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), stale, "file untouched");
    }

    #[test]
    fn profile_layer_row_id_occupies_the_collision_domain() {
        // Profile patches are applied BEFORE the home patch, so their row ids
        // share one namespace with it: generating `mcp-x` while a profile
        // already defines `mcp-x` would be a duplicate definition for dsh.
        let (_tmp, path) = patch_file("[]\n");
        let profile = path.parent().unwrap().join("profiles/web");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(
            profile.join("cordis.patch.yml"),
            "- insert:\n    - id: mcp-x\n      name: dsh-plugin-tool\n",
        )
        .unwrap();
        let err = deploy_mcp_server_dsh_cordis(&path, &stdio_entry("x")).unwrap_err();
        assert!(matches!(&err, HkError::Validation(m) if m.contains("mcp-x")), "got: {err:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]\n", "file untouched");
    }

    #[test]
    fn remove_on_a_wholly_absent_file_is_ok() {
        // Pins the writer-level idempotency (NotFound → "[]" synthesis in
        // read_and_split_home_patch) so it can't regress to an IO error —
        // independent of the dispatch-level exists() early return.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cordis.patch.yml");
        remove_mcp_server_dsh_cordis(&path, "anything").unwrap();
        assert!(!path.exists(), "no file conjured by a no-op removal");
    }
}
