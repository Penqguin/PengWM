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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_info_struct_size() {
        assert_eq!(std::mem::size_of::<SpaceInfo>(), 32);
    }

    #[test]
    fn functions_exist_with_correct_signatures() {
        fn _check_all_spaces(_f: fn() -> Vec<SpaceInfo>) {}
        fn _check_active_space(_f: fn(u32) -> Option<u8>) {}
        fn _check_switch_space(_f: fn(u32, u8)) {}
        _check_all_spaces(all_spaces);
        _check_active_space(active_space_for_display);
        _check_switch_space(switch_to_space);
    }
}
