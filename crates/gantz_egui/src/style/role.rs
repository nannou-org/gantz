//! The semantic colour slots of a style. See [`Role`].

use super::palette::{ColorRef, Palette, Swatch, Swatch::*};

/// Declare the roles once and derive [`Role`], [`Roles`] and their metadata
/// from the one list.
macro_rules! roles {
    ($($field:ident, $Variant:ident, $Group:ident, $label:literal, $help:literal, $default:expr;)*) => {
        /// A semantic colour slot of the UI. Each one selects a palette colour
        /// through [`Roles`].
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Role {
            $($Variant,)*
        }

        /// The palette colour that each [`Role`] selects.
        ///
        /// A role missing from a stored config takes its default.
        #[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
        #[serde(default)]
        pub struct Roles {
            $(pub $field: ColorRef,)*
        }

        impl Role {
            /// Every role, in display order.
            pub const ALL: &[Self] = &[$(Self::$Variant,)*];

            pub fn label(self) -> &'static str {
                match self {
                    $(Self::$Variant => $label,)*
                }
            }

            /// Where the role's colour appears.
            pub fn help(self) -> &'static str {
                match self {
                    $(Self::$Variant => $help,)*
                }
            }

            pub fn group(self) -> Group {
                match self {
                    $(Self::$Variant => Group::$Group,)*
                }
            }
        }

        impl Roles {
            pub fn get(&self, role: Role) -> ColorRef {
                match role {
                    $(Role::$Variant => self.$field,)*
                }
            }

            pub fn get_mut(&mut self, role: Role) -> &mut ColorRef {
                match role {
                    $(Role::$Variant => &mut self.$field,)*
                }
            }
        }

        impl Default for Roles {
            fn default() -> Self {
                Self {
                    $($field: $default,)*
                }
            }
        }
    };
}

roles! {
    scene, Scene, Surface, "scene",
        "The ground under the panes: the graph and the gaps between panes.",
        c(N0);
    canvas, Canvas, Surface, "canvas",
        "The background of text fields, plots and inspect views.",
        c(N0);
    panel, Panel, Surface, "pane",
        "The background of panes and tab bars.",
        c(N1);
    card, Card, Surface, "card",
        "The background of a group card, when group cards are nested.",
        c(N2);
    window, Window, Surface, "window",
        "The background of windows, menus and popups.",
        c(N1);
    faint, Faint, Surface, "faint",
        "A barely visible tint, for striped rows.",
        c(N7).with_opacity(0.02);
    code, Code, Surface, "code",
        "The background of inline code.",
        c(N2);
    border, Border, Surface, "border",
        "Outlines of cards, fields and windows.",
        c(N3);
    window_outline, WindowOutline, Surface, "window outline",
        "The outline around the app's window, when gantz draws its decorations.",
        c(N7).with_opacity(0.1);
    node, Node, Surface, "node",
        "The background of nodes, a step from the scene toward the panes.",
        c(N1).with_opacity(0.6);
    node_border, NodeBorder, Surface, "node outline",
        "The outline of nodes that are not selected.",
        c(N4).with_opacity(0.55);
    dot, Dot, Surface, "dot",
        "The dots of the scene grid.",
        c(N7).with_opacity(0.14);

    text, Text, Text, "text",
        "Labels and other plain text.",
        c(N5);
    weak_text, WeakText, Text, "weak text",
        "Hints, inactive tabs and other secondary text.",
        c(N4);
    strong_text, StrongText, Text, "strong text",
        "Headings, the active tab, and widgets while pressed.",
        c(N7);

    widget_fill, WidgetFill, Widget, "widget fill",
        "The background of buttons, dials and checkboxes.",
        c(N7).with_opacity(0.07);
    widget_text, WidgetText, Widget, "widget text",
        "The text and marks of buttons, dials and checkboxes.",
        c(N6);
    hover_fill, HoverFill, Widget, "hovered fill",
        "The background of a widget under the pointer.",
        c(N7).with_opacity(0.12);
    hover_stroke, HoverStroke, Widget, "hovered outline",
        "The outline of a widget under the pointer.",
        c(N4);
    hover_text, HoverText, Widget, "hovered text",
        "The text of a widget under the pointer or with an open menu.",
        c(N7);
    active_fill, ActiveFill, Widget, "pressed fill",
        "The background of a widget while pressed, or with an open menu.",
        c(N7).with_opacity(0.18);

    selection, Selection, Accent, "selection",
        "The background of selected text and selected items.",
        c(N7).with_opacity(0.14);
    selection_stroke, SelectionStroke, Accent, "selection outline",
        "The outline of selected nodes and items, and the text cursor.",
        c(N7);
    hyperlink, Hyperlink, Accent, "link",
        "Hyperlinks.",
        c(Blue);

    error, Error, Status, "error",
        "Errors and missing references.",
        c(Red);
    warning, Warning, Status, "warning",
        "Warnings and outdated references.",
        c(Orange);
    success, Success, Status, "success",
        "A live session or vault link.",
        c(Green);
    pending, Pending, Status, "pending",
        "A session or vault link while it connects.",
        c(Yellow);
    attention, Attention, Status, "attention",
        "A vault link that only the user can fix.",
        c(Purple);

    scrim, Scrim, Chrome, "scrim",
        "The dimming layer over a scene that is busy or awaits a drop.",
        c(N0).with_opacity(0.5);
    separator_hover, SeparatorHover, Chrome, "hovered separator",
        "The gap between panes while hovered or dragged.",
        c(N4);
}

/// A heading under which the style editor lists roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Surface,
    Text,
    Widget,
    Accent,
    Status,
    Chrome,
}

impl Role {
    /// The role this one is usually drawn over, to check contrast against.
    pub fn backdrop(self) -> Option<Role> {
        match self {
            Self::Text | Self::WeakText | Self::StrongText | Self::Hyperlink => Some(Self::Panel),
            Self::Error | Self::Warning | Self::Success | Self::Pending | Self::Attention => {
                Some(Self::Panel)
            }
            Self::WidgetText => Some(Self::WidgetFill),
            Self::HoverText => Some(Self::HoverFill),
            _ => None,
        }
    }

    /// The contrast ratio over [`Self::backdrop`] at which the role reads
    /// well. Secondary text may be fainter than body text.
    pub fn min_contrast(self) -> f32 {
        match self {
            Self::WeakText => 3.0,
            _ => 4.5,
        }
    }
}

impl Roles {
    /// The colour `role` selects from `palette`.
    pub fn resolve(&self, role: Role, palette: &Palette) -> egui::Color32 {
        self.get(role).resolve(palette)
    }
}

impl Group {
    pub const ALL: [Self; 6] = [
        Self::Surface,
        Self::Text,
        Self::Widget,
        Self::Accent,
        Self::Status,
        Self::Chrome,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Surface => "surfaces",
            Self::Text => "text",
            Self::Widget => "widgets",
            Self::Accent => "accents",
            Self::Status => "status",
            Self::Chrome => "chrome",
        }
    }
}

const fn c(swatch: Swatch) -> ColorRef {
    ColorRef::new(swatch)
}

#[cfg(test)]
mod tests {
    use super::{Role, Roles};

    #[test]
    fn get_mut_edits_the_role_get_reads() {
        let mut roles = Roles::default();
        for (i, &role) in Role::ALL.iter().enumerate() {
            roles.get_mut(role).opacity = i as f32;
        }
        for (i, &role) in Role::ALL.iter().enumerate() {
            assert_eq!(roles.get(role).opacity, i as f32);
        }
    }

    /// A stored config that predates a role still loads, with the role at its
    /// default.
    #[test]
    fn missing_roles_take_defaults() {
        let roles: Roles = ron::de::from_str("(text: (swatch: N6))").expect("parse roles");
        let mut expected = Roles::default();
        expected.text = super::c(super::N6);
        assert_eq!(roles, expected);
    }
}
