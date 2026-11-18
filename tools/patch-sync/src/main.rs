use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use kawari::{
    opcodes::{ClientZoneIpcType, ServerZoneIpcType},
    protocol_profile::{
        OpcodeOverride, ProtocolProfileFile, ZoneDownBaselineFile, detect_client_from_game_path,
        extract_zonedown_vtable_opcodes, resolve_game_path_optional, scan_sendpacket_call_sites,
    },
};

const HOUSING_SC_VARIANTS: &[&str] = &[
    "ZoneInit",
    "ActorControl",
    "SpawnObject",
    "SpawnNpc",
    "UpdateItem",
    "ContainerInfo",
    "HouseList",
    "HousingWardInfo",
    "FurnitureList",
    "OwnedHousing",
    "HousingInteriorDetails",
    "ApartmentList",
    "HousingOccupiedLandInfo",
    "HousingVacantLandInfo",
    "HousingEstateGreeting",
    "InteriorFurniturePlaced",
    "ExteriorFurniturePlaced",
    "FurnitureTranslatedForObserver",
    "UpdateMapMarkersForWardDivision",
    "HousingObjectDataValueSet",
    "HousingStainUpdated",
    "IndoorFurnitureMaterialUpdated",
    "HousingExteriorAppearanceUpdate",
    "BatchIndoorFurnitureTranslated",
    "BatchIndoorFurnitureSlotUpdate",
];

const HOUSING_CS_VARIANTS: &[&str] = &[
    "ClientTrigger",
    "FinishLoading",
    "ItemOperation",
    "PlaceFurniture",
    "TranslateFurniture",
    "BatchTranslateIndoorFurniture",
    "InteriorRemodelItemOperation",
    "ExteriorRemodelItemOperation",
    "DyeInformation",
    "UpdateEstateName",
    "UpdateEstateGreeting",
    "UpdateEstateProfile",
    "BuildEstateDeed",
    "UpdateAquarium",
    "UpdateOrchestrionPlaylist",
];

struct CliArgs {
    game_path: Option<PathBuf>,
    write_baseline: bool,
    write_profile: bool,
    verify: bool,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut args = CliArgs {
        game_path: None,
        write_baseline: false,
        write_profile: false,
        verify: false,
    };

    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--game-path" => {
                let p = it
                    .next()
                    .ok_or_else(|| "--game-path requires a directory argument".to_string())?;
                args.game_path = Some(PathBuf::from(p));
            }
            "--write-baseline" => args.write_baseline = true,
            "--write-profile" => args.write_profile = true,
            "--verify" => args.verify = true,
            "--help" | "-h" => {
                println!(
                    "Usage: foundation-patch-sync [--game-path <dir>] [--write-baseline] [--write-profile] [--verify]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("Unknown argument: {other}")),
        }
    }

    if !args.write_baseline && !args.write_profile && !args.verify {
        args.verify = true;
    }

    Ok(args)
}

fn find_repo_root() -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("resources/data/opcodes.yml").exists() {
        return cwd;
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest_dir.parent().and_then(|p| p.parent())
        && root.join("resources/data/opcodes.yml").exists()
    {
        return root.to_path_buf();
    }
    cwd
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::from(2);
        }
    };

    let game_path = if let Some(p) = args.game_path {
        p
    } else if Path::new(r"D:\SquareEnix\FINAL FANTASY XIV - A Realm Reborn\game\ffxiv_dx11.exe")
        .exists()
    {
        PathBuf::from(r"D:\SquareEnix\FINAL FANTASY XIV - A Realm Reborn\game")
    } else if let Some(p) = resolve_game_path_optional("") {
        p
    } else {
        eprintln!("Error: could not locate FFXIV game directory; pass --game-path <dir>");
        return ExitCode::from(2);
    };

    let exe_path = game_path.join("ffxiv_dx11.exe");
    let exe_bytes = match fs::read(&exe_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: failed to read {:?}: {e}", exe_path);
            return ExitCode::from(1);
        }
    };

    let exe_sha1 = sha1_smol::Sha1::from(&exe_bytes).digest().to_string();
    let detected = detect_client_from_game_path(&game_path);

    println!("Game Path          : {}", game_path.display());
    println!("Game Version       : {}", detected.game_version);
    println!("Boot Version       : {}", detected.boot_version);
    println!("Expansions         : {:?}", detected.expansion_versions);
    println!("LOBBY_GAME_VERSION : {}", detected.lobby_game_version);
    println!("ffxiv_dx11.exe len : {} bytes", exe_bytes.len());
    println!("ffxiv_dx11.exe sha1: {exe_sha1}");

    let zonedown = match extract_zonedown_vtable_opcodes(&exe_bytes) {
        Ok(z) => z,
        Err(e) => {
            eprintln!("Error: ZoneDown extraction failed: {e}");
            return ExitCode::from(1);
        }
    };
    println!(
        "ZoneDown vtable    : {} entries ({} unique 1:1 opcodes, {} total wired opcodes)",
        zonedown.total_vtable_entries,
        zonedown.vtable_to_opcode.len(),
        zonedown.opcode_to_vtable.len()
    );

    let sendpacket_sites = match scan_sendpacket_call_sites(&exe_bytes) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Warning: SendPacket call-site scan failed: {e}");
            BTreeMap::new()
        }
    };
    println!(
        "SendPacket sites   : {} distinct (opcode, seg_size) patterns",
        sendpacket_sites.len()
    );

    let repo_root = find_repo_root();
    let baseline_path = repo_root.join("resources/data/zonedown_baseline.yml");

    let mut baseline_opt: Option<ZoneDownBaselineFile> = fs::read_to_string(&baseline_path)
        .ok()
        .and_then(|raw| serde_yaml_ng::from_str(&raw).ok());

    if args.write_baseline {
        let mut vtable_offsets = BTreeMap::new();
        for &(name, opcode, _size) in ServerZoneIpcType::ALL_NAMED {
            if let Some(&vt) = zonedown.opcode_to_vtable.get(&opcode)
                && zonedown.vtable_to_opcode.get(&vt) == Some(&opcode)
            {
                vtable_offsets.insert(name.to_string(), vt);
            }
        }
        let baseline = ZoneDownBaselineFile {
            baseline_game_version: detected.game_version.clone(),
            total_vtable_entries: zonedown.total_vtable_entries,
            vtable_offsets,
        };
        if let Some(parent) = baseline_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let yaml = serde_yaml_ng::to_string(&baseline).expect("Failed to serialize baseline YAML");
        fs::write(&baseline_path, yaml).expect("Failed to write zonedown_baseline.yml");
        println!(
            "Wrote ZoneDown baseline ({} mapped ServerZone variants) -> {}",
            baseline.vtable_offsets.len(),
            baseline_path.display()
        );
        baseline_opt = Some(baseline);
    }

    let mut resolved_sc: BTreeMap<String, (u16, u32)> = BTreeMap::new();
    for &(name, default_op, size) in ServerZoneIpcType::ALL_NAMED {
        let remapped_op = baseline_opt
            .as_ref()
            .filter(|b| b.total_vtable_entries == zonedown.total_vtable_entries)
            .and_then(|b| b.vtable_offsets.get(name))
            .and_then(|vt| zonedown.vtable_to_opcode.get(vt))
            .copied()
            .unwrap_or(default_op);
        resolved_sc.insert(name.to_string(), (remapped_op, size));
    }

    if args.write_profile {
        let mut profile = ProtocolProfileFile {
            game_version: Some(detected.game_version.clone()),
            boot_version: Some(detected.boot_version.clone()),
            expansion_versions: Some(detected.expansion_versions.clone()),
            lobby_game_version: Some(detected.lobby_game_version),
            ..Default::default()
        };

        for (name, &(op, size)) in &resolved_sc {
            profile.server_zone.insert(
                name.clone(),
                OpcodeOverride::Detailed {
                    opcode: op,
                    size: Some(size),
                },
            );
        }
        for &(name, op, size) in ClientZoneIpcType::ALL_NAMED {
            profile.client_zone.insert(
                name.to_string(),
                OpcodeOverride::Detailed {
                    opcode: op,
                    size: Some(size),
                },
            );
        }

        let profile_path = repo_root
            .join("resources/profiles")
            .join(format!("{}.yml", detected.game_version));
        if let Some(parent) = profile_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let yaml = serde_yaml_ng::to_string(&profile).expect("Failed to serialize profile YAML");
        fs::write(&profile_path, yaml).expect("Failed to write protocol profile");
        println!(
            "Wrote protocol profile ({} S>C, {} C>S entries) -> {}",
            profile.server_zone.len(),
            profile.client_zone.len(),
            profile_path.display()
        );
    }

    if args.verify {
        println!(
            "\n=== Verification Against Binary ({}) ===",
            detected.game_version
        );
        let mut failures = 0usize;

        if detected.lobby_game_version == 0 {
            println!("  [FAIL] LOBBY_GAME_VERSION signature did not match");
            failures += 1;
        } else {
            println!(
                "  [OK] LOBBY_GAME_VERSION = {}",
                detected.lobby_game_version
            );
        }

        for &name in HOUSING_SC_VARIANTS {
            if let Some(&(op, size)) = resolved_sc.get(name) {
                if let Some(&vt) = zonedown.opcode_to_vtable.get(&op) {
                    println!(
                        "  [OK] S>C {name:<32} wire={op:4} (0x{op:04x}) size={size:4} vtable=0x{vt:04x}"
                    );
                } else {
                    println!(
                        "  [FAIL] S>C {name:<32} wire={op:4} NOT PRESENT in ZoneDown dispatcher"
                    );
                    failures += 1;
                }
            } else {
                println!("  [FAIL] S>C {name:<32} missing from ServerZoneIpcType");
                failures += 1;
            }
        }

        let mut cs_opcode_counts: BTreeMap<u16, usize> = BTreeMap::new();
        for (&(op, _), &cnt) in &sendpacket_sites {
            *cs_opcode_counts.entry(op).or_default() += cnt;
        }

        let cs_map: BTreeMap<&str, (u16, u32)> = ClientZoneIpcType::ALL_NAMED
            .iter()
            .map(|&(name, op, sz)| (name, (op, sz)))
            .collect();

        for &name in HOUSING_CS_VARIANTS {
            if let Some(&(op, size)) = cs_map.get(name) {
                let seg_size = size + 16;
                let exact_hits = sendpacket_sites.get(&(op, seg_size)).copied().unwrap_or(0);
                let any_hits = cs_opcode_counts.get(&op).copied().unwrap_or(0);
                if exact_hits > 0 {
                    println!(
                        "  [OK] C>S {name:<32} wire={op:4} seg={seg_size:3} ({exact_hits} exact SendPacket site(s))"
                    );
                } else if any_hits > 0 {
                    println!(
                        "  [OK] C>S {name:<32} wire={op:4} seg={seg_size:3} ({any_hits} opcode hit(s) in SendPacket)"
                    );
                } else {
                    println!(
                        "  [WARN] C>S {name:<32} wire={op:4} seg={seg_size:3} not matched by generic SendPacket imm heuristic"
                    );
                }
            } else {
                println!("  [FAIL] C>S {name:<32} missing from ClientZoneIpcType");
                failures += 1;
            }
        }

        if failures > 0 {
            eprintln!("\nVerification FAILED with {failures} error(s).");
            return ExitCode::from(1);
        }
        println!("\nVerification PASSED (0 failures).");
    }

    ExitCode::SUCCESS
}
