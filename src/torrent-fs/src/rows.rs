#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub permissions: Option<u32>,
    pub extra: Vec<String>,
}

pub const TICKED: &str = ic_plugin_api::IC_CELL_TICKED;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnKind {
    #[default]
    Text,
    Check,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnSpec {
    pub key: String,
    pub title: String,
    pub width: Option<i32>,
    pub kind: ColumnKind,
}
