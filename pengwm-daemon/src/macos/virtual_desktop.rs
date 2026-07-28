pub struct SpaceInfo {
    pub display_id: u32,
    pub space_index: u8,
    pub display_uuid: String,
}

pub fn all_spaces() -> Vec<SpaceInfo> {
    log::warn!("virtual_desktop::all_spaces not yet implemented (requires CGSPrivate)");
    Vec::new()
}

pub fn active_space_for_display(_display_id: u32) -> Option<u8> {
    None
}

pub fn switch_to_space(_display_id: u32, _space_index: u8) {
    log::warn!("virtual_desktop::switch_to_space not yet implemented (requires CGSPrivate)");
}
