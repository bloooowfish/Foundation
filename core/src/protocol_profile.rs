use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::LazyLock,
};

use parking_lot::RwLock;
use physis::existing_dirs::find_existing_game_dirs;
use serde_derive::{Deserialize, Serialize};

use crate::{
    common::ItemOperationKind,
    constants::{
        LOBBY_GAME_VERSION, SUPPORTED_BOOT_VERSION, SUPPORTED_EXPAC_VERSIONS,
        SUPPORTED_GAME_VERSION,
    },
    opcodes::{
        ClientChatIpcType, ClientLobbyIpcType, ClientZoneIpcType, CustomIpcType, ServerChatIpcType,
        ServerLobbyIpcType, ServerZoneIpcType,
    },
};

/// Runtime protocol & patch-resilience configuration.
#[derive(Serialize, Deserialize)]
pub struct ProtocolConfig {
    /// Optional explicit path to a YAML opcode/version override profile.
    #[serde(default)]
    pub profile_path: String,

    /// Automatically read `.ver` files and scan `LOBBY_GAME_VERSION` from `filesystem.game_path`.
    #[serde(default = "ProtocolConfig::default_true")]
    pub auto_detect_client_version: bool,

    /// Automatically extract `ZoneDown` vtable opcodes from `ffxiv_dx11.exe` when baseline matches.
    #[serde(default = "ProtocolConfig::default_true")]
    pub auto_extract_zonedown_opcodes: bool,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            profile_path: String::new(),
            auto_detect_client_version: true,
            auto_extract_zonedown_opcodes: true,
        }
    }
}

impl ProtocolConfig {
    fn default_true() -> bool {
        true
    }
}

impl ItemOperationKind {
    /// Converts this operation kind to its wire u32 value relative to runtime `ClientZoneIpcType::ItemOperation`.
    pub fn to_wire(self) -> u32 {
        match self {
            Self::UnkDiscard | Self::UnkTrading2 | Self::UnkGearSetEquipped => self as u32,
            other => ClientZoneIpcType::ItemOperation.get_opcode() as u32 + (other as u32),
        }
    }

    /// Parses a wire u32 value relative to runtime `ClientZoneIpcType::ItemOperation`.
    pub fn from_wire(raw: u32) -> Option<Self> {
        let base = ClientZoneIpcType::ItemOperation.get_opcode() as u32;
        if let Some(rel) = raw.checked_sub(base)
            && (1..=19).contains(&rel)
            && let Some(kind) = Self::from_repr(rel)
        {
            return Some(kind);
        }

        match raw {
            0x90 => Some(Self::UnkDiscard),
            256 => Some(Self::UnkTrading2),
            567 => Some(Self::UnkGearSetEquipped),
            _ => None,
        }
    }
}

impl binrw::BinRead for ItemOperationKind {
    type Args<'a> = ();

    fn read_options<R: std::io::Read + std::io::Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<Self> {
        let pos = reader.stream_position()?;
        let raw = u32::read_options(reader, endian, args)?;
        Self::from_wire(raw).ok_or_else(|| binrw::Error::AssertFail {
            pos,
            message: format!("Unknown ItemOperationKind wire value {raw}"),
        })
    }
}

impl binrw::BinWrite for ItemOperationKind {
    type Args<'a> = ();

    fn write_options<W: std::io::Write + std::io::Seek>(
        &self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        self.to_wire().write_options(writer, endian, args)
    }
}

/// IPC domain backed by a generated opcode enum in [`crate::opcodes`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum OpcodeDomain {
    ServerZone,
    ClientZone,
    ServerLobby,
    ClientLobby,
    ServerChat,
    ClientChat,
    Custom,
}

impl OpcodeDomain {
    pub const ALL: [Self; 7] = [
        Self::ServerZone,
        Self::ClientZone,
        Self::ServerLobby,
        Self::ClientLobby,
        Self::ServerChat,
        Self::ClientChat,
        Self::Custom,
    ];

    pub const fn as_index(self) -> usize {
        match self {
            Self::ServerZone => 0,
            Self::ClientZone => 1,
            Self::ServerLobby => 2,
            Self::ClientLobby => 3,
            Self::ServerChat => 4,
            Self::ClientChat => 5,
            Self::Custom => 6,
        }
    }

    pub const fn yaml_key(self) -> &'static str {
        match self {
            Self::ServerZone => "ServerZoneIpcType",
            Self::ClientZone => "ClientZoneIpcType",
            Self::ServerLobby => "ServerLobbyIpcType",
            Self::ClientLobby => "ClientLobbyIpcType",
            Self::ServerChat => "ServerChatIpcType",
            Self::ClientChat => "ClientChatIpcType",
            Self::Custom => "CustomIpcType",
        }
    }
}

/// Optional size + required opcode override parsed from YAML profile files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpcodeOverride {
    OpcodeOnly(u16),
    Detailed {
        opcode: u16,
        #[serde(default)]
        size: Option<u32>,
    },
}

impl OpcodeOverride {
    pub const fn opcode(&self) -> u16 {
        match self {
            Self::OpcodeOnly(opcode) => *opcode,
            Self::Detailed { opcode, .. } => *opcode,
        }
    }

    pub const fn size(&self) -> Option<u32> {
        match self {
            Self::OpcodeOnly(_) => None,
            Self::Detailed { size, .. } => *size,
        }
    }
}

/// Runtime protocol override profile stored in `resources/profiles/<ver>.yml`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProtocolProfileFile {
    #[serde(default)]
    pub game_version: Option<String>,
    #[serde(default)]
    pub boot_version: Option<String>,
    #[serde(default)]
    pub expansion_versions: Option<Vec<String>>,
    #[serde(default)]
    pub lobby_game_version: Option<u32>,

    #[serde(default, rename = "ServerZoneIpcType")]
    pub server_zone: BTreeMap<String, OpcodeOverride>,
    #[serde(default, rename = "ClientZoneIpcType")]
    pub client_zone: BTreeMap<String, OpcodeOverride>,
    #[serde(default, rename = "ServerLobbyIpcType")]
    pub server_lobby: BTreeMap<String, OpcodeOverride>,
    #[serde(default, rename = "ClientLobbyIpcType")]
    pub client_lobby: BTreeMap<String, OpcodeOverride>,
    #[serde(default, rename = "ServerChatIpcType")]
    pub server_chat: BTreeMap<String, OpcodeOverride>,
    #[serde(default, rename = "ClientChatIpcType")]
    pub client_chat: BTreeMap<String, OpcodeOverride>,
    #[serde(default, rename = "CustomIpcType")]
    pub custom: BTreeMap<String, OpcodeOverride>,
}

/// Baseline vtable mapping generated from a verified patch binary (used to auto-remap minor patches).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ZoneDownBaselineFile {
    pub baseline_game_version: String,
    pub total_vtable_entries: usize,
    /// `ServerZoneIpcType` variant name -> ZoneDown dispatcher vtable byte offset (e.g. `0x10`).
    #[serde(default)]
    pub vtable_offsets: BTreeMap<String, u32>,
}

/// Client version metadata read directly from a local FFXIV installation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedClientProfile {
    pub game_version: String,
    pub boot_version: String,
    pub expansion_versions: Vec<String>,
    pub lobby_game_version: u32,
}

impl Default for DetectedClientProfile {
    fn default() -> Self {
        Self {
            game_version: SUPPORTED_GAME_VERSION.to_string(),
            boot_version: SUPPORTED_BOOT_VERSION.to_string(),
            expansion_versions: SUPPORTED_EXPAC_VERSIONS
                .iter()
                .map(|v| (*v).to_string())
                .collect(),
            lobby_game_version: LOBBY_GAME_VERSION,
        }
    }
}

#[derive(Clone)]
struct DomainOpcodeTable {
    defaults: &'static [(&'static str, u16, u32)],
    by_wire: Box<[u16; 65536]>,
    by_index: Vec<(u16, u32)>,
    name_to_index: HashMap<&'static str, usize>,
}

impl DomainOpcodeTable {
    fn from_defaults(defaults: &'static [(&'static str, u16, u32)]) -> Self {
        let mut by_wire = Box::new([u16::MAX; 65536]);
        let mut by_index = Vec::with_capacity(defaults.len());
        let mut name_to_index = HashMap::with_capacity(defaults.len());

        for (idx, &(name, opcode, size)) in defaults.iter().enumerate() {
            by_wire[opcode as usize] = idx as u16;
            by_index.push((opcode, size));
            name_to_index.insert(name, idx);
        }

        Self {
            defaults,
            by_wire,
            by_index,
            name_to_index,
        }
    }

    fn reset(&mut self) {
        self.by_wire.fill(u16::MAX);
        for (idx, &(_, opcode, size)) in self.defaults.iter().enumerate() {
            self.by_wire[opcode as usize] = idx as u16;
            self.by_index[idx] = (opcode, size);
        }
    }

    fn apply_overrides(
        &mut self,
        domain: OpcodeDomain,
        overrides: &BTreeMap<String, OpcodeOverride>,
    ) -> usize {
        if overrides.is_empty() {
            return 0;
        }

        let mut changed = 0;
        for (name, ov) in overrides {
            let Some(&idx) = self.name_to_index.get(name.as_str()) else {
                tracing::warn!(
                    "Ignoring unknown opcode override {}.{name} -> {}",
                    domain.yaml_key(),
                    ov.opcode()
                );
                continue;
            };

            let (old_opcode, old_size) = self.by_index[idx];
            let new_opcode = ov.opcode();
            let new_size = ov.size().unwrap_or(old_size);
            if old_opcode != new_opcode || old_size != new_size {
                changed += 1;
            }
            self.by_index[idx] = (new_opcode, new_size);
        }

        self.rebuild_wire_table(domain);
        changed
    }

    fn rebuild_wire_table(&mut self, domain: OpcodeDomain) {
        self.by_wire.fill(u16::MAX);
        for (idx, &(opcode, _)) in self.by_index.iter().enumerate() {
            let existing = self.by_wire[opcode as usize];
            if existing != u16::MAX {
                let prev_name = self.defaults[existing as usize].0;
                let curr_name = self.defaults[idx].0;
                tracing::warn!(
                    "Runtime opcode collision in {} for wire opcode {}: {} overwritten by {}",
                    domain.yaml_key(),
                    opcode,
                    prev_name,
                    curr_name
                );
            }
            self.by_wire[opcode as usize] = idx as u16;
        }
    }
}

#[derive(Clone)]
pub struct ProtocolRuntimeState {
    domains: [DomainOpcodeTable; 7],
    client: DetectedClientProfile,
}

impl ProtocolRuntimeState {
    fn from_compiled_defaults() -> Self {
        Self {
            domains: [
                DomainOpcodeTable::from_defaults(ServerZoneIpcType::ALL_NAMED),
                DomainOpcodeTable::from_defaults(ClientZoneIpcType::ALL_NAMED),
                DomainOpcodeTable::from_defaults(ServerLobbyIpcType::ALL_NAMED),
                DomainOpcodeTable::from_defaults(ClientLobbyIpcType::ALL_NAMED),
                DomainOpcodeTable::from_defaults(ServerChatIpcType::ALL_NAMED),
                DomainOpcodeTable::from_defaults(ClientChatIpcType::ALL_NAMED),
                DomainOpcodeTable::from_defaults(CustomIpcType::ALL_NAMED),
            ],
            client: DetectedClientProfile::default(),
        }
    }

    fn reset(&mut self) {
        for domain in &mut self.domains {
            domain.reset();
        }
        self.client = DetectedClientProfile::default();
    }

    pub fn apply_profile(&mut self, profile: &ProtocolProfileFile) -> usize {
        if let Some(ref game_version) = profile.game_version {
            self.client.game_version = game_version.clone();
        }
        if let Some(ref boot_version) = profile.boot_version {
            self.client.boot_version = boot_version.clone();
        }
        if let Some(ref expacs) = profile.expansion_versions {
            self.client.expansion_versions = expacs.clone();
        }
        if let Some(lobby_ver) = profile.lobby_game_version {
            self.client.lobby_game_version = lobby_ver;
        }

        let mut total = 0;
        total += self.domains[OpcodeDomain::ServerZone.as_index()]
            .apply_overrides(OpcodeDomain::ServerZone, &profile.server_zone);
        total += self.domains[OpcodeDomain::ClientZone.as_index()]
            .apply_overrides(OpcodeDomain::ClientZone, &profile.client_zone);
        total += self.domains[OpcodeDomain::ServerLobby.as_index()]
            .apply_overrides(OpcodeDomain::ServerLobby, &profile.server_lobby);
        total += self.domains[OpcodeDomain::ClientLobby.as_index()]
            .apply_overrides(OpcodeDomain::ClientLobby, &profile.client_lobby);
        total += self.domains[OpcodeDomain::ServerChat.as_index()]
            .apply_overrides(OpcodeDomain::ServerChat, &profile.server_chat);
        total += self.domains[OpcodeDomain::ClientChat.as_index()]
            .apply_overrides(OpcodeDomain::ClientChat, &profile.client_chat);
        total += self.domains[OpcodeDomain::Custom.as_index()]
            .apply_overrides(OpcodeDomain::Custom, &profile.custom);
        total
    }
}

static PROTOCOL_STATE: LazyLock<RwLock<ProtocolRuntimeState>> =
    LazyLock::new(|| RwLock::new(ProtocolRuntimeState::from_compiled_defaults()));

#[cfg(test)]
thread_local! {
    static TEST_PROTOCOL_OVERRIDE: std::cell::RefCell<Option<ProtocolRuntimeState>> =
        const { std::cell::RefCell::new(None) };
}

fn with_active_state<R>(f: impl FnOnce(&ProtocolRuntimeState) -> R) -> R {
    #[cfg(test)]
    if TEST_PROTOCOL_OVERRIDE.with(|cell| cell.borrow().is_some()) {
        return TEST_PROTOCOL_OVERRIDE.with(|cell| {
            let borrow = cell.borrow();
            f(borrow.as_ref().unwrap())
        });
    }

    let guard = PROTOCOL_STATE.read();
    f(&guard)
}

/// Resolves a wire opcode in O(1) into the compiled variant index for `domain`.
#[inline]
pub fn resolve_wire_index(domain: OpcodeDomain, raw: u16) -> Option<usize> {
    with_active_state(|state| {
        let idx = state.domains[domain.as_index()].by_wire[raw as usize];
        if idx == u16::MAX {
            None
        } else {
            Some(idx as usize)
        }
    })
}

/// Resolves the active runtime wire opcode for a compiled variant index in `domain`.
#[inline]
pub fn resolve_opcode_by_index(domain: OpcodeDomain, idx: usize) -> u16 {
    with_active_state(|state| state.domains[domain.as_index()].by_index[idx].0)
}

/// Resolves the active runtime payload size (excluding IPC header) for a compiled variant index.
#[inline]
pub fn resolve_size_by_index(domain: OpcodeDomain, idx: usize) -> u32 {
    with_active_state(|state| state.domains[domain.as_index()].by_index[idx].1)
}

/// Applies runtime opcode overrides to `domain` and returns the number of changed entries.
pub fn apply_domain_overrides(
    domain: OpcodeDomain,
    overrides: &BTreeMap<String, OpcodeOverride>,
) -> usize {
    let mut state = PROTOCOL_STATE.write();
    state.domains[domain.as_index()].apply_overrides(domain, overrides)
}

/// Applies an entire parsed [`ProtocolProfileFile`] to runtime state.
pub fn apply_protocol_profile(profile: &ProtocolProfileFile) -> usize {
    let mut state = PROTOCOL_STATE.write();
    state.apply_profile(profile)
}

/// Resets runtime opcodes and client version metadata back to compiled defaults.
pub fn reset_to_compiled_defaults() {
    let mut state = PROTOCOL_STATE.write();
    state.reset();
}

/// Active `LOBBY_GAME_VERSION` used for Blowfish key derivation.
pub fn active_lobby_game_version() -> u32 {
    with_active_state(|state| state.client.lobby_game_version)
}

/// Active `SUPPORTED_GAME_VERSION` string (e.g. `2026.11.03.0000.0000`).
pub fn active_supported_game_version() -> String {
    with_active_state(|state| state.client.game_version.clone())
}

/// Active `SUPPORTED_BOOT_VERSION` string.
pub fn active_supported_boot_version() -> String {
    with_active_state(|state| state.client.boot_version.clone())
}

/// Active `SUPPORTED_EXPAC_VERSIONS` list.
pub fn active_supported_expac_versions() -> Vec<String> {
    with_active_state(|state| state.client.expansion_versions.clone())
}

/// Active full [`DetectedClientProfile`] snapshot.
pub fn active_client_profile() -> DetectedClientProfile {
    with_active_state(|state| state.client.clone())
}

/// Finds a game installation directory if `configured` is empty, mirroring `FilesystemConfig::game_path`.
pub fn resolve_game_path_optional(configured: &str) -> Option<std::path::PathBuf> {
    if !configured.is_empty() {
        let p = Path::new(configured);
        if p.exists() {
            return Some(p.to_path_buf());
        }
        return None;
    }

    find_existing_game_dirs()
        .last()
        .map(|d| Path::new(&d.path).join("game"))
        .filter(|p| p.exists())
}

/// Reads `.ver` files and extracts `LOBBY_GAME_VERSION` from `<game_path>/ffxiv_dx11.exe`.
pub fn detect_client_from_game_path(game_path: &Path) -> DetectedClientProfile {
    let mut profile = DetectedClientProfile::default();

    let game_ver_path = game_path.join("ffxivgame.ver");
    if let Ok(ver) = std::fs::read_to_string(&game_ver_path) {
        let trimmed = ver.trim();
        if !trimmed.is_empty() {
            profile.game_version = trimmed.to_string();
        }
    }

    if let Some(parent) = game_path.parent() {
        let boot_ver_path = parent.join("boot").join("ffxivboot.ver");
        if let Ok(ver) = std::fs::read_to_string(&boot_ver_path) {
            let trimmed = ver.trim();
            if !trimmed.is_empty() {
                profile.boot_version = trimmed.to_string();
            }
        }
    }

    let mut expacs = Vec::new();
    for idx in 1..=32 {
        let ex_name = format!("ex{idx}");
        let ex_ver_path = game_path
            .join("sqpack")
            .join(&ex_name)
            .join(format!("{ex_name}.ver"));
        if let Ok(ver) = std::fs::read_to_string(&ex_ver_path) {
            let trimmed = ver.trim();
            if !trimmed.is_empty() {
                expacs.push(trimmed.to_string());
                continue;
            }
        }
        break;
    }
    if !expacs.is_empty() {
        profile.expansion_versions = expacs;
    }

    let exe_path = game_path.join("ffxiv_dx11.exe");
    if let Ok(exe_bytes) = std::fs::read(&exe_path)
        && let Some(lobby_ver) = extract_lobby_game_version(&exe_bytes)
    {
        profile.lobby_game_version = lobby_ver;
    }

    profile
}

#[derive(Clone, Copy, Debug)]
struct PeSection {
    virtual_address: u32,
    virtual_size: u32,
    raw_ptr: u32,
    raw_size: u32,
    characteristics: u32,
}

impl PeSection {
    const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;

    fn is_executable(&self) -> bool {
        (self.characteristics & Self::IMAGE_SCN_MEM_EXECUTE) != 0
    }
}

fn parse_pe_sections(exe_bytes: &[u8]) -> Result<Vec<PeSection>, String> {
    if exe_bytes.len() < 0x40 || &exe_bytes[0..2] != b"MZ" {
        return Err("Not a valid PE executable (missing MZ header)".to_string());
    }
    let e_lfanew = u32::from_le_bytes(exe_bytes[0x3C..0x40].try_into().unwrap()) as usize;
    if exe_bytes.len() < e_lfanew + 24 || &exe_bytes[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        return Err("Invalid PE signature".to_string());
    }

    let coff = e_lfanew + 4;
    let num_sections =
        u16::from_le_bytes(exe_bytes[coff + 2..coff + 4].try_into().unwrap()) as usize;
    let opt_hdr_size =
        u16::from_le_bytes(exe_bytes[coff + 16..coff + 18].try_into().unwrap()) as usize;
    let sections_start = coff + 20 + opt_hdr_size;

    if exe_bytes.len() < sections_start + num_sections * 40 {
        return Err("Truncated PE section table".to_string());
    }

    let mut sections = Vec::with_capacity(num_sections);
    for i in 0..num_sections {
        let base = sections_start + i * 40;
        let virtual_size = u32::from_le_bytes(exe_bytes[base + 8..base + 12].try_into().unwrap());
        let virtual_address =
            u32::from_le_bytes(exe_bytes[base + 12..base + 16].try_into().unwrap());
        let raw_size = u32::from_le_bytes(exe_bytes[base + 16..base + 20].try_into().unwrap());
        let raw_ptr = u32::from_le_bytes(exe_bytes[base + 20..base + 24].try_into().unwrap());
        let characteristics =
            u32::from_le_bytes(exe_bytes[base + 36..base + 40].try_into().unwrap());

        sections.push(PeSection {
            virtual_address,
            virtual_size,
            raw_ptr,
            raw_size,
            characteristics,
        });
    }

    Ok(sections)
}

fn rva_to_offset(sections: &[PeSection], rva: u32, file_len: usize) -> Option<usize> {
    for sec in sections {
        let span = sec.virtual_size.max(sec.raw_size);
        if rva >= sec.virtual_address && rva < sec.virtual_address.saturating_add(span) {
            let delta = rva - sec.virtual_address;
            if delta < sec.raw_size {
                let offset = sec.raw_ptr as usize + delta as usize;
                if offset < file_len {
                    return Some(offset);
                }
            }
        }
    }
    None
}

fn offset_to_rva(sections: &[PeSection], offset: usize) -> Option<u32> {
    let off_u32 = u32::try_from(offset).ok()?;
    for sec in sections {
        if off_u32 >= sec.raw_ptr && off_u32 < sec.raw_ptr.saturating_add(sec.raw_size) {
            return Some(sec.virtual_address + (off_u32 - sec.raw_ptr));
        }
    }
    None
}

/// Scans `ffxiv_dx11.exe` for `C7 46 ?? <u32> E8 ?? ?? ?? ?? 48 3B F3` to recover `LOBBY_GAME_VERSION`.
pub fn extract_lobby_game_version(exe_bytes: &[u8]) -> Option<u32> {
    let sections = parse_pe_sections(exe_bytes).ok()?;
    for sec in sections.iter().filter(|s| s.is_executable()) {
        let start = sec.raw_ptr as usize;
        let end = (start + sec.raw_size as usize).min(exe_bytes.len());
        if end.saturating_sub(start) < 15 {
            continue;
        }
        let slice = &exe_bytes[start..end];
        for i in 0..=slice.len() - 15 {
            if slice[i] == 0xC7
                && slice[i + 1] == 0x46
                && slice[i + 7] == 0xE8
                && slice[i + 12] == 0x48
                && slice[i + 13] == 0x3B
                && slice[i + 14] == 0xF3
            {
                let val = u32::from_le_bytes(slice[i + 3..i + 7].try_into().ok()?);
                if (1000..=100_000).contains(&val) {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Extracted `ZoneDown` interface vtable mapping from a `ffxiv_dx11.exe` binary.
#[derive(Clone, Debug)]
pub struct ZoneDownExtraction {
    /// Total switch cases mapped to `ZoneDown` interface vtable slots.
    pub total_vtable_entries: usize,
    /// `vtable_offset` -> unique wire opcode (when only 1 opcode targets this vtable slot).
    pub vtable_to_opcode: BTreeMap<u32, u16>,
    /// Wire opcode -> `vtable_offset`.
    pub opcode_to_vtable: BTreeMap<u16, u32>,
}

/// Pure-Rust signature scanner for the `ZoneDown` S>C vtable dispatcher in `ffxiv_dx11.exe`.
///
/// Mirrors `opcodediff/vtable_diff.py` without requiring radare2 or IDA at runtime.
pub fn extract_zonedown_vtable_opcodes(exe_bytes: &[u8]) -> Result<ZoneDownExtraction, String> {
    const SIG: [u8; 13] = [
        0x49, 0x8B, 0x40, 0x10, // mov rax, [r8+10h]
        0x4C, 0x8B, 0x50, 0x38, // mov r10, [rax+38h]
        0x41, 0x0F, 0xB7, 0x42, 0x02, // movzx eax, word ptr [r10+2]
    ];

    let sections = parse_pe_sections(exe_bytes)?;

    let mut sig_offset = None;
    'outer: for sec in sections.iter().filter(|s| s.is_executable()) {
        let start = sec.raw_ptr as usize;
        let end = (start + sec.raw_size as usize).min(exe_bytes.len());
        if end.saturating_sub(start) < 64 {
            continue;
        }
        if let Some(rel) = exe_bytes[start..end]
            .windows(SIG.len())
            .position(|w| w == SIG)
        {
            sig_offset = Some(start + rel);
            break 'outer;
        }
    }

    let sig_pos =
        sig_offset.ok_or_else(|| "ZoneDown dispatcher signature not found".to_string())?;
    let mut p = sig_pos + SIG.len();
    if p + 32 > exe_bytes.len() {
        return Err("Truncated ZoneDown dispatcher header".to_string());
    }

    // Parse add/sub applied to `eax` before bounds check.
    let opcode_delta: i32 = if exe_bytes[p] == 0x83 && exe_bytes[p + 1] == 0xC0 {
        let d = (exe_bytes[p + 2] as i8) as i32;
        p += 3;
        d
    } else if exe_bytes[p] == 0x05 {
        let d = i32::from_le_bytes(exe_bytes[p + 1..p + 5].try_into().unwrap());
        p += 5;
        d
    } else if exe_bytes[p] == 0x83 && exe_bytes[p + 1] == 0xE8 {
        let d = -((exe_bytes[p + 2] as i8) as i32);
        p += 3;
        d
    } else if exe_bytes[p] == 0x2D {
        let d = -i32::from_le_bytes(exe_bytes[p + 1..p + 5].try_into().unwrap());
        p += 5;
        d
    } else {
        0
    };

    // Parse switch bounds check `cmp eax, max_case`.
    let max_case: u32 = if exe_bytes[p] == 0x3D {
        let m = u32::from_le_bytes(exe_bytes[p + 1..p + 5].try_into().unwrap());
        p += 5;
        m
    } else if exe_bytes[p] == 0x83 && exe_bytes[p + 1] == 0xF8 {
        let m = exe_bytes[p + 2] as u32;
        p += 3;
        m
    } else {
        return Err(format!(
            "Unexpected instruction at ZoneDown cmp bounds check (bytes: {:02X?})",
            &exe_bytes[p..p + 8]
        ));
    };

    if max_case > 4096 {
        return Err(format!("Implausible ZoneDown max_case={max_case}"));
    }

    // Locate `mov reg, [base + index*4 + table_rva]` within next 36 bytes.
    let mut table_rva = None;
    let search_end = (p + 36).min(exe_bytes.len().saturating_sub(8));
    for i in p..search_end {
        let has_rex = (0x40..=0x4F).contains(&exe_bytes[i]);
        let op_idx = if has_rex { i + 1 } else { i };
        if op_idx + 6 < exe_bytes.len() && exe_bytes[op_idx] == 0x8B {
            let modrm = exe_bytes[op_idx + 1];
            let sib = exe_bytes[op_idx + 2];
            if (modrm & 0xC7) == 0x84 && (sib & 0xC0) == 0x80 {
                let rva = u32::from_le_bytes(exe_bytes[op_idx + 3..op_idx + 7].try_into().unwrap());
                if rva_to_offset(&sections, rva, exe_bytes.len()).is_some() {
                    table_rva = Some(rva);
                    break;
                }
            }
        }
    }

    let table_rva =
        table_rva.ok_or_else(|| "Could not locate ZoneDown jump table RVA".to_string())?;
    let table_off = rva_to_offset(&sections, table_rva, exe_bytes.len())
        .ok_or_else(|| format!("Invalid jump table RVA 0x{table_rva:x}"))?;

    let num_cases = max_case as usize + 1;
    if table_off + num_cases * 4 > exe_bytes.len() {
        return Err("Truncated ZoneDown jump table".to_string());
    }

    let mut target_to_opcodes: BTreeMap<u32, Vec<u16>> = BTreeMap::new();
    for case_idx in 0..num_cases {
        let entry_off = table_off + case_idx * 4;
        let case_rva = u32::from_le_bytes(exe_bytes[entry_off..entry_off + 4].try_into().unwrap());
        let wire_opcode = (case_idx as i32 - opcode_delta) as u16;
        target_to_opcodes
            .entry(case_rva)
            .or_default()
            .push(wire_opcode);
    }

    let mut vtable_to_opcode: BTreeMap<u32, u16> = BTreeMap::new();
    let mut opcode_to_vtable: BTreeMap<u16, u32> = BTreeMap::new();
    let mut total_vtable_entries = 0usize;

    for (case_rva, wire_opcodes) in target_to_opcodes {
        if wire_opcodes.len() > 10 {
            continue;
        }
        let Some(case_off) = rva_to_offset(&sections, case_rva, exe_bytes.len()) else {
            continue;
        };
        let end = (case_off + 24).min(exe_bytes.len());
        let block = &exe_bytes[case_off..end];

        let Some(vtable_offset) = parse_zonedown_case_vtable_offset(block) else {
            continue;
        };

        total_vtable_entries += 1;
        for &wire_op in &wire_opcodes {
            opcode_to_vtable.insert(wire_op, vtable_offset);
        }
        if wire_opcodes.len() == 1 {
            vtable_to_opcode.insert(vtable_offset, wire_opcodes[0]);
        }
    }

    if total_vtable_entries == 0 {
        return Err("No ZoneDown vtable dispatch cases decoded".to_string());
    }

    Ok(ZoneDownExtraction {
        total_vtable_entries,
        vtable_to_opcode,
        opcode_to_vtable,
    })
}

fn parse_zonedown_case_vtable_offset(block: &[u8]) -> Option<u32> {
    let has_mov = block.windows(3).any(|w| w == [0x48, 0x8B, 0x01]);
    let has_lea = block.windows(4).any(|w| w == [0x4D, 0x8D, 0x4A, 0x10]);
    if !has_mov || !has_lea {
        return None;
    }

    for i in 0..block.len() {
        let j = if block[i] == 0x48 { i + 1 } else { i };
        if j + 1 < block.len() && block[j] == 0xFF {
            match block[j + 1] {
                0x20 => return Some(0),
                0x60 if j + 2 < block.len() => return Some((block[j + 2] as i8) as u32),
                0xA0 if j + 5 < block.len() => {
                    return Some(u32::from_le_bytes(block[j + 2..j + 6].try_into().ok()?));
                }
                _ => {}
            }
        }
    }
    None
}

/// Scans `ffxiv_dx11.exe` for `Client::Network::NetworkModuleProxy.SendPacket` call sites,
/// returning `(wire_opcode, segment_size)` -> occurrence count.
pub fn scan_sendpacket_call_sites(exe_bytes: &[u8]) -> Result<BTreeMap<(u16, u32), usize>, String> {
    // Stable prologue of `Client::Network::NetworkModuleProxy.SendPacket`:
    // 48 89 5C 24 ?? 55 56 57 41 54 41 56 41 57 48 81 EC ?? ?? ?? ?? ...
    // Let's also detect it reliably via `Client::Game::GameMain.ExecuteCommand` (`0x235`? no, dynamic):
    // Find `lea r8/rdx`? Actually in `SendPacket`: `mov [rsp+8], rbx; push rbp; ... 48 8B 05 .. 48 33 C4 48 89 84 24`
    // Even simpler: scan all `call rel32` targets in `.text` preceded within 48 bytes by BOTH a u32 opcode `100..2000`
    // store AND a u64 segment_size store (`8..4096` multiple of 8 + 16), and pick the dominant target!
    let sections = parse_pe_sections(exe_bytes)?;

    let mut target_hits: HashMap<u32, Vec<(u16, u32)>> = HashMap::new();
    for sec in sections.iter().filter(|s| s.is_executable()) {
        let start = sec.raw_ptr as usize;
        let end = (start + sec.raw_size as usize).min(exe_bytes.len());
        if end.saturating_sub(start) < 32 {
            continue;
        }
        let slice = &exe_bytes[start..end];
        for rel in 0..slice.len() - 5 {
            if slice[rel] != 0xE8 {
                continue;
            }
            let disp = i32::from_le_bytes(slice[rel + 1..rel + 5].try_into().unwrap());
            let next_off = start + rel + 5;
            let Some(next_rva) = offset_to_rva(&sections, next_off) else {
                continue;
            };
            let target_rva = (next_rva as i64 + disp as i64) as u32;
            if rva_to_offset(&sections, target_rva, exe_bytes.len()).is_none() {
                continue;
            }

            // Look backwards up to 96 bytes for stack descriptor stores.
            let win_start = rel.saturating_sub(96);
            if let Some((opcode, seg_size)) = inspect_descriptor_stores(&slice[win_start..rel]) {
                target_hits
                    .entry(target_rva)
                    .or_default()
                    .push((opcode, seg_size));
            }
        }
    }

    let (_, pairs) = target_hits
        .into_iter()
        .max_by_key(|(_, v)| v.len())
        .ok_or_else(|| "No SendPacket call sites detected".to_string())?;

    let mut counts: BTreeMap<(u16, u32), usize> = BTreeMap::new();
    for pair in pairs {
        *counts.entry(pair).or_default() += 1;
    }
    Ok(counts)
}

fn inspect_descriptor_stores(window: &[u8]) -> Option<(u16, u32)> {
    // Match `mov dword ptr [rsp/rbp+disp], imm32` (`C7 44 24 imm8 imm32`, `C7 84 24 imm32 imm32`, `C7 45 imm8 imm32`, `C7 85 imm32 imm32`)
    // and `mov qword ptr [rsp/rbp+disp], imm32` (`48 C7 ...`).
    let mut opcodes = Vec::new();
    let mut seg_sizes = Vec::new();

    let mut i = 0;
    while i + 6 < window.len() {
        let is_qword = window[i] == 0x48 && window[i + 1] == 0xC7;
        let is_dword = window[i] == 0xC7;
        if !is_qword && !is_dword {
            i += 1;
            continue;
        }
        let op_off = if is_qword { i + 2 } else { i + 1 };
        if op_off >= window.len() {
            break;
        }
        let imm_off = match window[op_off] {
            0x44 if op_off + 1 < window.len() && window[op_off + 1] == 0x24 => op_off + 3,
            0x84 if op_off + 1 < window.len() && window[op_off + 1] == 0x24 => op_off + 6,
            0x45 => op_off + 2,
            0x85 => op_off + 5,
            _ => {
                i += 1;
                continue;
            }
        };
        if imm_off + 4 > window.len() {
            i += 1;
            continue;
        }
        let imm = u32::from_le_bytes(window[imm_off..imm_off + 4].try_into().unwrap());
        if is_qword {
            if (16..=4096).contains(&imm) && imm % 8 == 0 {
                seg_sizes.push(imm);
            }
        } else if (80..=2000).contains(&imm) {
            opcodes.push(imm as u16);
        }
        i = imm_off + 4;
    }

    if let (Some(&op), Some(&seg)) = (opcodes.last(), seg_sizes.last()) {
        Some((op, seg))
    } else {
        None
    }
}

#[cfg(feature = "server")]
static INIT_ONCE: std::sync::Once = std::sync::Once::new();

/// Initializes runtime protocol detection, ZoneDown vtable remapping, and optional profile loading.
#[cfg(feature = "server")]
pub fn init_runtime_protocol(config: &crate::config::Config) {
    INIT_ONCE.call_once(|| {
        apply_runtime_protocol_from_config(config);
    });
}

#[cfg(feature = "server")]
fn apply_runtime_protocol_from_config(config: &crate::config::Config) {
    let game_path = resolve_game_path_optional(&config.filesystem.game_path);

    if config.protocol.auto_detect_client_version
        && let Some(ref gp) = game_path
    {
        let detected = detect_client_from_game_path(gp);
        tracing::info!(
            "Detected client profile from {:?}: game={}, boot={}, lobby_ver={}, expacs={}",
            gp,
            detected.game_version,
            detected.boot_version,
            detected.lobby_game_version,
            detected.expansion_versions.len()
        );
        let mut state = PROTOCOL_STATE.write();
        state.client = detected;
    }

    // Optional automatic ZoneDown S>C remapping using baseline vtable map.
    if config.protocol.auto_extract_zonedown_opcodes
        && let Some(ref gp) = game_path
    {
        let exe_path = gp.join("ffxiv_dx11.exe");
        if let Some(baseline_path) = locate_resource_file(
            &config.filesystem.additional_resource_paths,
            "data/zonedown_baseline.yml",
        ) && let Ok(baseline_yaml) = std::fs::read_to_string(&baseline_path)
            && let Ok(baseline) = serde_yaml_ng::from_str::<ZoneDownBaselineFile>(&baseline_yaml)
            && let Ok(exe_bytes) = std::fs::read(&exe_path)
        {
            match extract_zonedown_vtable_opcodes(&exe_bytes) {
                Ok(extracted)
                    if extracted.total_vtable_entries == baseline.total_vtable_entries =>
                {
                    let mut auto_overrides = BTreeMap::new();
                    for (name, vtable_off) in &baseline.vtable_offsets {
                        if let Some(&wire_op) = extracted.vtable_to_opcode.get(vtable_off) {
                            auto_overrides
                                .insert(name.clone(), OpcodeOverride::OpcodeOnly(wire_op));
                        }
                    }
                    let changed = apply_domain_overrides(OpcodeDomain::ServerZone, &auto_overrides);
                    tracing::info!(
                        "ZoneDown vtable auto-sync matched {} entries (updated {} ServerZone opcodes)",
                        extracted.total_vtable_entries,
                        changed
                    );
                }
                Ok(extracted) => {
                    tracing::warn!(
                        "ZoneDown vtable entry count changed ({} -> {}), skipping auto-remap; run foundation-patch-sync or provide a profile file",
                        baseline.total_vtable_entries,
                        extracted.total_vtable_entries
                    );
                }
                Err(err) => {
                    tracing::warn!("ZoneDown vtable extraction skipped: {err}");
                }
            }
        }
    }

    // Explicit profile file overrides everything above if present.
    let explicit_profile_path = if !config.protocol.profile_path.is_empty() {
        Some(std::path::PathBuf::from(&config.protocol.profile_path))
    } else {
        let ver = active_supported_game_version();
        locate_resource_file(
            &config.filesystem.additional_resource_paths,
            &format!("profiles/{ver}.yml"),
        )
    };

    if let Some(profile_path) = explicit_profile_path
        && profile_path.exists()
    {
        match std::fs::read_to_string(&profile_path) {
            Ok(raw) => match serde_yaml_ng::from_str::<ProtocolProfileFile>(&raw) {
                Ok(profile) => {
                    let changed = apply_protocol_profile(&profile);
                    tracing::info!(
                        "Loaded protocol profile {:?} ({} opcode/size overrides applied)",
                        profile_path,
                        changed
                    );
                }
                Err(err) => {
                    tracing::error!("Failed to parse protocol profile {:?}: {err}", profile_path);
                }
            },
            Err(err) => {
                tracing::error!("Failed to read protocol profile {:?}: {err}", profile_path);
            }
        }
    }
}

#[cfg(feature = "server")]
fn locate_resource_file(additional_paths: &[String], rel: &str) -> Option<std::path::PathBuf> {
    for dir in additional_paths {
        let candidate = Path::new(dir).join(rel);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let builtin = Path::new("resources").join(rel);
    if builtin.exists() {
        return Some(builtin);
    }
    None
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::{BinRead, BinWrite};

    use super::*;
    use crate::common::ItemOperationKind;
    use crate::ipc::zone::ServerZoneIpcSegment;
    use crate::packet::{IpcSegmentHeader, ReadWriteIpcSegment, generate_encryption_key};

    fn with_local_protocol_state<R>(profile: &ProtocolProfileFile, f: impl FnOnce() -> R) -> R {
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                TEST_PROTOCOL_OVERRIDE.with(|cell| {
                    *cell.borrow_mut() = None;
                });
            }
        }

        let mut state = ProtocolRuntimeState::from_compiled_defaults();
        state.apply_profile(profile);
        TEST_PROTOCOL_OVERRIDE.with(|cell| {
            *cell.borrow_mut() = Some(state);
        });
        let _guard = Guard;
        f()
    }

    #[test]
    fn runtime_opcode_and_version_override_works_without_races() {
        let mut profile = ProtocolProfileFile {
            game_version: Some("2099.01.01.0000.0000".to_string()),
            boot_version: Some("2099.01.01.0000.0001".to_string()),
            expansion_versions: Some(vec!["2099.01.01.0000.0000".to_string()]),
            lobby_game_version: Some(8000),
            ..Default::default()
        };
        profile.server_zone.insert(
            "HouseList".to_string(),
            OpcodeOverride::Detailed {
                opcode: 901,
                size: Some(1216),
            },
        );
        profile
            .client_zone
            .insert("ItemOperation".to_string(), OpcodeOverride::OpcodeOnly(400));

        with_local_protocol_state(&profile, || {
            assert_eq!(active_supported_game_version(), "2099.01.01.0000.0000");
            assert_eq!(active_supported_boot_version(), "2099.01.01.0000.0001");
            assert_eq!(active_lobby_game_version(), 8000);
            assert_ne!(
                generate_encryption_key(&[0, 0, 0, 0], "foobar"),
                [
                    147, 75, 201, 132, 255, 202, 150, 130, 59, 31, 162, 205, 133, 248, 14, 1
                ]
            );

            // ServerZoneIpcType round-trip with overridden opcode 901
            let wire_read = ServerZoneIpcType::from_wire_opcode(901);
            assert_eq!(wire_read, ServerZoneIpcType::HouseList);
            assert_eq!(wire_read.get_opcode(), 901);
            assert_eq!(
                ServerZoneIpcType::from_wire_opcode(791),
                ServerZoneIpcType::Unknown(791)
            );

            let mut seg_buf = Cursor::new(Vec::new());
            let hdr: crate::packet::ServerIpcSegmentHeader<ServerZoneIpcType> =
                IpcSegmentHeader::from_opcode(ServerZoneIpcType::HouseList);
            hdr.write_le(&mut seg_buf).unwrap();
            let raw = seg_buf.into_inner();
            assert_eq!(u16::from_le_bytes([raw[2], raw[3]]), 901);

            // ClientZoneIpcType::ItemOperation -> dynamically shifts ItemOperationKind wire encoding!
            let mut op_buf = Cursor::new(Vec::new());
            ItemOperationKind::Move.write_le(&mut op_buf).unwrap();
            let raw_op = op_buf.into_inner();
            assert_eq!(u32::from_le_bytes(raw_op.clone().try_into().unwrap()), 408);

            let mut cur = Cursor::new(&raw_op);
            let decoded = ItemOperationKind::read_le(&mut cur).unwrap();
            assert_eq!(decoded, ItemOperationKind::Move);
        });

        // Outside the local override, defaults remain intact!
        assert_eq!(active_lobby_game_version(), LOBBY_GAME_VERSION);
        assert_eq!(
            ServerZoneIpcType::from_wire_opcode(791),
            ServerZoneIpcType::HouseList
        );
    }

    #[test]
    fn check_ipc_segment_size_remains_consistent() {
        let mut profile = ProtocolProfileFile::default();
        profile.server_zone.insert(
            "HousingInteriorDetails".to_string(),
            OpcodeOverride::Detailed {
                opcode: 999,
                size: Some(56),
            },
        );
        with_local_protocol_state(&profile, || {
            let seg = ServerZoneIpcSegment::new(
                crate::ipc::zone::ServerZoneIpcData::HousingInteriorDetails(Default::default()),
            );
            assert_eq!(seg.get_opcode(), 999);
            assert_eq!(seg.calc_size(), crate::packet::IPC_HEADER_SIZE + 56);
        });
    }
}
