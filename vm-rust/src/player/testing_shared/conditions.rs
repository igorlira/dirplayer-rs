use std::future::IntoFuture;
use std::pin::Pin;
use crate::director::static_datum::StaticDatum;
use super::{now_ms, TestHarness, DEFAULT_TIMEOUT_SECS};

// --- Query & check types ---

/// How to find a sprite on stage.
pub enum SpriteQuery {
    MemberName(String),
    MemberPrefix(String),
    Number(usize),
}

impl std::fmt::Display for SpriteQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpriteQuery::MemberName(n) => write!(f, "member '{}'", n),
            SpriteQuery::MemberPrefix(p) => write!(f, "member prefix '{}'", p),
            SpriteQuery::Number(n) => write!(f, "sprite({})", n),
        }
    }
}

/// What to check once a sprite is found.
pub enum SpriteCheck {
    Exists,
    Visible(f64),
}

/// A condition that `step_until` polls each frame.
pub enum StepCondition {
    Sprite { query: SpriteQuery, check: SpriteCheck },
    ExprEquals { expr: String, expected: StaticDatum },
    ExprTruthy { expr: String },
}

impl std::fmt::Display for StepCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepCondition::Sprite { query, check } => {
                write!(f, "{}", query)?;
                match check {
                    SpriteCheck::Exists => write!(f, " exists"),
                    SpriteCheck::Visible(min) => write!(f, " visible >= {:.0}%", min * 100.0),
                }
            }
            StepCondition::ExprEquals { expr, expected } =>
                write!(f, "{} == {:?}", expr, expected),
            StepCondition::ExprTruthy { expr } =>
                write!(f, "{} is truthy", expr),
        }
    }
}

// --- Condition builders ---

/// Start building a sprite condition.
///
/// ```ignore
/// sprite().member("Logo").visible(1.0)
/// sprite().member_prefix("login_").exists()
/// sprite().number(5).visible(0.5)
/// ```
pub fn sprite() -> SpriteConditionBuilder {
    SpriteConditionBuilder { query: None }
}

/// Start building a datum condition.
///
/// ```ignore
/// datum("ilk(gCore)").equals(StaticDatum::Symbol("instance".into()))
/// datum("gReady").is_truthy()
/// ```
pub fn datum(expr: &str) -> DatumConditionBuilder {
    DatumConditionBuilder { expr: expr.to_string() }
}

pub struct SpriteConditionBuilder {
    query: Option<SpriteQuery>,
}

/// Allow `sprite().member("x")` to be passed directly to `click_sprite()` etc.
impl From<SpriteConditionBuilder> for SpriteQuery {
    fn from(builder: SpriteConditionBuilder) -> Self {
        builder.query.expect("sprite query needs .member(), .member_prefix(), or .number()")
    }
}

impl SpriteConditionBuilder {
    pub fn member(mut self, name: &str) -> Self {
        self.query = Some(SpriteQuery::MemberName(name.to_string()));
        self
    }

    pub fn member_prefix(mut self, prefix: &str) -> Self {
        self.query = Some(SpriteQuery::MemberPrefix(prefix.to_string()));
        self
    }

    pub fn number(mut self, n: usize) -> Self {
        self.query = Some(SpriteQuery::Number(n));
        self
    }

    pub fn visible(self, min_visibility: f64) -> StepCondition {
        StepCondition::Sprite {
            query: self.query.expect("sprite condition needs a query (.member(), .member_prefix(), or .number())"),
            check: SpriteCheck::Visible(min_visibility),
        }
    }

    pub fn exists(self) -> StepCondition {
        StepCondition::Sprite {
            query: self.query.expect("sprite condition needs a query (.member(), .member_prefix(), or .number())"),
            check: SpriteCheck::Exists,
        }
    }
}

pub struct DatumConditionBuilder {
    expr: String,
}

impl DatumConditionBuilder {
    pub fn equals(self, expected: StaticDatum) -> StepCondition {
        StepCondition::ExprEquals { expr: self.expr, expected }
    }

    pub fn is_truthy(self) -> StepCondition {
        StepCondition::ExprTruthy { expr: self.expr }
    }
}

// --- StepUntilBuilder ---

/// Builder returned by `TestHarness::step_until()`. Implements `IntoFuture`
/// so it can be `.await`ed directly, or customized with `.timeout()` first.
pub struct StepUntilBuilder<'a, H: TestHarness + ?Sized> {
    harness: &'a mut H,
    condition: StepCondition,
    timeout_secs: f64,
}

impl<'a, H: TestHarness + ?Sized> StepUntilBuilder<'a, H> {
    pub(super) fn new(harness: &'a mut H, condition: StepCondition) -> Self {
        StepUntilBuilder {
            harness,
            condition,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    /// Override the default timeout (in seconds).
    pub fn timeout(mut self, secs: f64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

impl<'a, H: TestHarness + ?Sized> IntoFuture for StepUntilBuilder<'a, H> {
    type Output = Result<(), String>;
    type IntoFuture = Pin<Box<dyn std::future::Future<Output = Result<(), String>> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let deadline_ms = now_ms() + self.timeout_secs * 1000.0;
            let mut frames = 0usize;
            while now_ms() < deadline_ms {
                if check_condition(self.harness, &self.condition).await {
                    return Ok(());
                }
                if !self.harness.step_frame().await {
                    return Err(format!("Movie stopped while waiting for {}", self.condition));
                }
                frames += 1;
            }
            let detail = condition_detail(self.harness, &self.condition).await;
            Err(format!(
                "{} not met after {:.1}s / {} frames{}",
                self.condition, self.timeout_secs, frames, detail
            ))
        })
    }
}

/// Check whether a condition is currently satisfied.
async fn check_condition(harness: &(impl TestHarness + ?Sized), condition: &StepCondition) -> bool {
    match condition {
        StepCondition::Sprite { query, check } => {
            let sprite_num = match harness.find_sprite(query) {
                Some(n) => n,
                None => return false,
            };
            match check {
                SpriteCheck::Exists => true,
                SpriteCheck::Visible(min) => harness.sprite_visibility(sprite_num).await >= *min,
            }
        }
        StepCondition::ExprEquals { expr, expected } => {
            harness.eval_datum(expr).await.ok().as_ref() == Some(expected)
        }
        StepCondition::ExprTruthy { expr } => {
            matches!(harness.eval_datum(expr).await.ok(), Some(StaticDatum::Int(v)) if v != 0)
        }
    }
}

/// Build a detail string for timeout error messages.
async fn condition_detail(harness: &(impl TestHarness + ?Sized), condition: &StepCondition) -> String {
    match condition {
        StepCondition::Sprite { query, check } => {
            if let Some(sprite_num) = harness.find_sprite(query) {
                match check {
                    SpriteCheck::Exists => String::new(),
                    SpriteCheck::Visible(min) => {
                        let vis = harness.sprite_visibility(sprite_num).await;
                        format!(" (at {:.1}%, need {:.0}%)", vis * 100.0, min * 100.0)
                    }
                }
            } else {
                " (sprite not found)".to_string()
            }
        }
        StepCondition::ExprEquals { expr, .. } => {
            match harness.eval_datum(expr).await {
                Ok(actual) => format!(" (actual: {:?})", actual),
                Err(e) => format!(" (eval error: {})", e),
            }
        }
        StepCondition::ExprTruthy { expr } => {
            match harness.eval_datum(expr).await {
                Ok(actual) => format!(" (actual: {:?})", actual),
                Err(e) => format!(" (eval error: {})", e),
            }
        }
    }
}
