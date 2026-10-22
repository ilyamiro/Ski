//! Quick Look: Space shows the selected file in a floating preview. The window itself lives in
//! `app/quick_look.rs`; this type only gives the feature its switch in Settings → Plugins.
use super::Plugin;
use crate::i18n::tr;
use crate::ui::icons;

pub struct QuickLook;

impl Plugin for QuickLook {
    fn id(&self) -> &'static str {
        "quick_look"
    }

    fn name(&self) -> &'static str {
        tr("Quick Look")
    }

    fn description(&self) -> &'static str {
        tr("Press Space to preview text, code, images and details of the selected file")
    }

    fn icon(&self) -> &'static str {
        icons::VISIBILITY
    }
}
