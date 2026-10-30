//! Borrow a live interpreter for staging helpers, without constructing a Session.
use super::*;

pub(super) struct StageContext<'a> {
    pub interp: &'a mut Interp,
    pub id: ContextID,
}
impl StageContext<'_> {
    pub fn scalar(&mut self, name: &str) -> Result<String> {
        check_supported(self.interp, self.id)?;
        let value = self.interp.with_global_scope(|interp| interp.scalar(name));
        check_supported(self.interp, self.id)?;
        value.map(|value| value.to_string()).map_err(failure)
    }
}
