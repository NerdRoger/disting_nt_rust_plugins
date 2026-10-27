//! Preset schema and incremental loading, independent of host JSON pointers.
#![forbid(unsafe_code)]

use super::controls::EditMode;
#[cfg(test)]
use core::ffi::CStr;

pub(super) use distingnt_api::api::json::{Reader, Writer};

#[derive(Clone, Copy)]
pub(super) struct State {
    pub edit_mode: EditMode,
    pub editable: bool,
}

impl State {
    pub fn save(&self, writer: &mut impl Writer) {
        writer.boolean_member(c"BoundsEditMode", self.edit_mode.is_bounds());
        writer.boolean_member(c"Editable", self.editable);
    }

    // Missing members retain their current values. Successful earlier changes
    // survive a later parse error, matching the original incremental loader.
    pub fn load(&mut self, reader: &mut impl Reader) -> bool {
        let Some(count) = reader.member_count() else {
            return false;
        };
        for _ in 0..count {
            if reader.matches(c"BoundsEditMode") {
                let mut bounds = false;
                if !reader.boolean(&mut bounds) {
                    return false;
                }
                self.edit_mode = EditMode::from_bounds(bounds);
            } else if reader.matches(c"Editable") {
                if !reader.boolean(&mut self.editable) {
                    return false;
                }
            } else if !reader.skip() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    #[derive(Default)]
    struct Json {
        members: Vec<(&'static CStr, Option<bool>)>,
        position: usize,
        fail_count: bool,
        fail_skip: bool,
    }

    impl Reader for Json {
        fn member_count(&mut self) -> Option<i32> {
            (!self.fail_count).then_some(self.members.len() as i32)
        }
        fn matches(&mut self, name: &CStr) -> bool {
            self.members[self.position].0 == name
        }
        fn boolean(&mut self, value: &mut bool) -> bool {
            let Some(parsed) = self.members[self.position].1 else {
                return false;
            };
            *value = parsed;
            self.position += 1;
            true
        }
        fn skip(&mut self) -> bool {
            if self.fail_skip {
                return false;
            }
            self.position += 1;
            true
        }
    }

    impl Writer for Json {
        fn boolean_member(&mut self, name: &CStr, value: bool) {
            // The schema has exactly these two static keys.
            let key = if name == c"BoundsEditMode" {
                c"BoundsEditMode"
            } else {
                assert_eq!(name, c"Editable");
                c"Editable"
            };
            self.members.push((key, Some(value)));
        }
    }

    fn initial() -> State {
        State {
            edit_mode: EditMode::Bounds,
            editable: true,
        }
    }

    #[test]
    fn all_flag_combinations_round_trip_in_original_key_order() {
        for bounds in [false, true] {
            for editable in [false, true] {
                let original = State {
                    edit_mode: EditMode::from_bounds(bounds),
                    editable,
                };
                let mut json = Json::default();
                original.save(&mut json);
                assert_eq!(
                    json.members.as_slice(),
                    &[
                        (c"BoundsEditMode", Some(bounds)),
                        (c"Editable", Some(editable))
                    ]
                );
                let mut restored = initial();
                assert!(restored.load(&mut json));
                assert_eq!(restored.edit_mode, original.edit_mode);
                assert_eq!(restored.editable, editable);
            }
        }
    }

    #[test]
    fn missing_unknown_reordered_and_duplicate_members_preserve_schema() {
        let mut state = initial();
        assert!(state.load(&mut Json::default()));
        assert!(state.editable && state.edit_mode.is_bounds());
        let mut json = Json::default();
        json.members.extend([
            (c"Unknown", None),
            (c"Editable", Some(false)),
            (c"BoundsEditMode", Some(false)),
            (c"Editable", Some(true)),
        ]);
        assert!(state.load(&mut json));
        assert!(!state.edit_mode.is_bounds() && state.editable);
        let mut json = Json::default();
        json.members.push((c"Editable", Some(false)));
        assert!(state.load(&mut json));
        assert!(!state.edit_mode.is_bounds() && !state.editable);
    }

    #[test]
    fn malformed_boolean_retains_successful_earlier_changes() {
        for bad_name in [c"BoundsEditMode", c"Editable"] {
            let mut state = initial();
            let mut json = Json::default();
            json.members
                .extend([(c"Editable", Some(false)), (bad_name, None)]);
            assert!(!state.load(&mut json));
            assert!(!state.editable);
            assert!(state.edit_mode.is_bounds());
            assert_eq!(json.position, 1);
        }
    }

    #[test]
    fn malformed_object_and_unskippable_members_report_failure() {
        let mut state = initial();
        assert!(!state.load(&mut Json {
            fail_count: true,
            ..Json::default()
        }));
        let mut json = Json {
            fail_skip: true,
            ..Json::default()
        };
        json.members.push((c"Unknown", None));
        assert!(!state.load(&mut json));
        assert!(state.editable && state.edit_mode.is_bounds());
    }
}
