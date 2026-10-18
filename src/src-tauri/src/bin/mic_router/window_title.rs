//! 本体管理に合わせた製品・数値版の共通タイトル。
use crate::config::{APP_NAME, APP_VERSION};
pub struct WindowTitle;
impl WindowTitle {
    pub fn create(page: &str) -> String {
        if page.is_empty() {
            format!("{APP_NAME} - ver {APP_VERSION}")
        } else {
            format!("{page} - {APP_NAME} - ver {APP_VERSION}")
        }
    }
}
