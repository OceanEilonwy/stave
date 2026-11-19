//! A kind's rules that read `wall(cut)` (grammar, "Walls").
//!
//! The wall round a cut is known only once every cut of the design is
//! made, after the kinds' rules run, so such a rule is worked out here as
//! far as it can be (its message and the other side of its comparison)
//! and measured by the fit check (`stave-check`), which reports it at the
//! joint's or hardware's line like any failed rule.

use stave_lang::ast::BinaryOp;
use stave_model::defs::{CutName, RExpr, RExprKind, RequireDef};
use stave_model::ids::{KindId, RequireIndex};
use stave_model::solved::{CutSource, WallCompare, WallRule};

use crate::eval::Env;
use crate::joints::round_length;
use crate::{Solved, Solver};

/// The comparison a wall rule makes, with the wall on the left: the cut
/// it measures, how it compares, and the other side. `None` for a rule
/// that doesn't read `wall(cut)`. The resolver has checked that a rule
/// reading it is one comparison with `wall(cut)` alone on one side.
pub(crate) fn wall_comparison(condition: &RExpr) -> Option<(&CutName, WallCompare, &RExpr)> {
    let RExprKind::Binary { op, lhs, rhs } = &condition.kind else {
        return None;
    };
    match (&lhs.kind, &rhs.kind) {
        (RExprKind::Wall(cut), _) => Some((cut, compare(*op, false)?, rhs)),
        (_, RExprKind::Wall(cut)) => Some((cut, compare(*op, true)?, lhs)),
        _ => None,
    }
}

/// A comparison operator as a [`WallCompare`], turned round when the wall
/// is on the right (`3mm <= wall(hole)` is `wall(hole) >= 3mm`).
fn compare(op: BinaryOp, flipped: bool) -> Option<WallCompare> {
    Some(match (op, flipped) {
        (BinaryOp::Ge, false) | (BinaryOp::Le, true) => WallCompare::AtLeast,
        (BinaryOp::Gt, false) | (BinaryOp::Lt, true) => WallCompare::MoreThan,
        (BinaryOp::Le, false) | (BinaryOp::Ge, true) => WallCompare::AtMost,
        (BinaryOp::Lt, false) | (BinaryOp::Gt, true) => WallCompare::LessThan,
        (BinaryOp::Eq, _) => WallCompare::Exactly,
        (BinaryOp::Ne, _) => WallCompare::Not,
        (
            BinaryOp::Or
            | BinaryOp::And
            | BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::In,
            _,
        ) => return None,
    })
}

impl Solver<'_> {
    /// When `require` reads `wall(cut)`, works out its message and the
    /// other side of its comparison for `source` and keeps it for the fit
    /// check; `true` when it did (the rule is not evaluated now).
    pub(crate) fn defer_wall_rule(
        &mut self,
        source: CutSource,
        kind: KindId,
        index: RequireIndex,
        require: &RequireDef,
        env: &Env<'_>,
    ) -> Solved<bool> {
        let Some((cut, compare, other)) = wall_comparison(&require.condition) else {
            return Ok(false);
        };
        let needs = round_length(self.eval_length(other, env)?)?;
        let message = self.format_text(&require.message, env)?;
        self.wall_rules.push(WallRule {
            source,
            kind,
            require: index,
            name: require.decl.name.clone(),
            message,
            cut: cut.clone(),
            compare,
            needs,
        });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wall_on_the_right_turns_the_comparison_round() {
        assert_eq!(compare(BinaryOp::Le, true), Some(WallCompare::AtLeast));
        assert_eq!(compare(BinaryOp::Ge, false), Some(WallCompare::AtLeast));
        assert_eq!(compare(BinaryOp::Lt, true), Some(WallCompare::MoreThan));
        assert_eq!(compare(BinaryOp::Gt, true), Some(WallCompare::LessThan));
        assert_eq!(compare(BinaryOp::Eq, true), Some(WallCompare::Exactly));
        assert_eq!(compare(BinaryOp::Add, false), None);
    }
}
