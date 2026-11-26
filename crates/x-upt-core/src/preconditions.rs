//! Precondition evaluation and active-dependency lock guards for business methods and fronts.

use serde::Serialize;

use crate::domain::{
    AppRecord, AuthMethod, BusinessMethod, EnvelopeProfileSpec, FrontName, FrontRecord,
    MethodsState,
};
use crate::error::AppError;
use crate::imgs::{ImageMeta, PhotoScene};

/// Counts of imported images per scene.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SceneCounts {
    /// Count of `article_cover` images.
    pub article_cover: usize,
    /// Count of `avatar` images.
    pub avatar: usize,
    /// Count of `banner` images.
    pub banner: usize,
}

impl SceneCounts {
    /// Total imported image count across all scenes.
    #[must_use]
    pub fn total(self) -> usize {
        self.article_cover + self.avatar + self.banner
    }

    /// Computes scene counts from image metadata records.
    #[must_use]
    pub fn from_images(images: &[ImageMeta]) -> Self {
        let mut counts = Self::default();
        for img in images {
            match img.scene {
                PhotoScene::ArticleCover => counts.article_cover += 1,
                PhotoScene::Avatar => counts.avatar += 1,
                PhotoScene::Banner => counts.banner += 1,
            }
        }
        counts
    }
}

/// Readiness evaluation for a single business method switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MethodStatus {
    /// Canonical wire method name.
    pub method: String,
    /// Display label.
    pub label: String,
    /// Whether the switch is currently enabled.
    pub enabled: bool,
    /// Whether preconditions are satisfied to turn the switch on.
    pub can_enable: bool,
    /// Missing precondition descriptions when `can_enable` is `false`.
    pub missing_preconditions: Vec<String>,
}

/// Evaluates missing preconditions for enabling `method`.
#[must_use]
pub fn missing_for_method(
    method: BusinessMethod,
    scenes: SceneCounts,
    posts_count: usize,
) -> Vec<String> {
    let mut missing = Vec::new();
    match method {
        BusinessMethod::PostsSearch => {
            if posts_count == 0 {
                missing.push("需先导入至少 1 条帖子数据".to_string());
            }
        }
        BusinessMethod::PhotosRender => {
            if scenes.article_cover == 0 {
                missing.push("缺少 article_cover 场景基准图片".to_string());
            }
            if scenes.avatar == 0 {
                missing.push("缺少 avatar 场景基准图片".to_string());
            }
            if scenes.banner == 0 {
                missing.push("缺少 banner 场景基准图片".to_string());
            }
        }
        BusinessMethod::ImgdefCompare => {
            if scenes.total() == 0 {
                missing.push("需先导入至少 1 张基准图片".to_string());
            }
        }
    }
    missing
}

/// Computes [`MethodStatus`] views for all 3 business methods.
#[must_use]
pub fn evaluate_methods(
    state: MethodsState,
    scenes: SceneCounts,
    posts_count: usize,
) -> Vec<MethodStatus> {
    BusinessMethod::all()
        .into_iter()
        .map(|m| {
            let missing = missing_for_method(m, scenes, posts_count);
            MethodStatus {
                method: m.as_str().to_string(),
                label: m.label().to_string(),
                enabled: state.get(m),
                can_enable: missing.is_empty(),
                missing_preconditions: missing,
            }
        })
        .collect()
}

/// Context passed when evaluating whether a front can be enabled.
pub struct FrontEvalInput<'a> {
    /// Control-plane port to prevent port collision.
    pub admin_port: u16,
    /// All 15 front records.
    pub all_fronts: &'a [FrontRecord],
    /// Current business method switch states.
    pub methods: MethodsState,
    /// Registered caller applications.
    pub apps: &'a [AppRecord],
    /// Profiles configured for this front (if envelope front).
    pub profiles: &'a [EnvelopeProfileSpec],
    /// Validation errors for envelope profiles/secrets (empty if valid).
    pub envelope_errors: &'a [String],
}

/// Evaluates missing preconditions for enabling `front`.
#[must_use]
pub fn missing_for_front(front: &FrontRecord, ctx: &FrontEvalInput<'_>) -> Vec<String> {
    let mut missing = Vec::new();
    check_port_and_tls(front, ctx, &mut missing);
    if front.name.is_envelope() {
        check_envelope_front(front, ctx, &mut missing);
    } else {
        check_core_front(front, ctx, &mut missing);
    }
    missing
}

fn check_port_and_tls(front: &FrontRecord, ctx: &FrontEvalInput<'_>, missing: &mut Vec<String>) {
    if front.host.trim().is_empty() || front.port == 0 {
        missing.push("监听地址或端口无效".to_string());
    }
    if front.port == ctx.admin_port {
        missing.push(format!("端口 {} 与控制台端口冲突", front.port));
    }
    for other in ctx.all_fronts {
        if other.name != front.name && other.enabled && ports_collide(front, other) {
            missing.push(format!(
                "监听地址 {}:{} 已被 {} 占用",
                front.host,
                front.port,
                other.name.as_str()
            ));
        }
    }
    if front.tls.enabled {
        let cert_ok = front
            .tls
            .cert_pem
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty());
        let key_ok = front
            .tls
            .key_pem
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty());
        if !cert_ok || !key_ok {
            missing.push("已开启 TLS，但尚未配置服务端证书或私钥 PEM".to_string());
        }
    }
}

fn ports_collide(a: &FrontRecord, b: &FrontRecord) -> bool {
    if a.port != b.port {
        return false;
    }
    let ah = a.host.trim();
    let bh = b.host.trim();
    ah == bh || ah == "0.0.0.0" || bh == "0.0.0.0" || ah == "::" || bh == "::"
}

fn check_core_front(front: &FrontRecord, ctx: &FrontEvalInput<'_>, missing: &mut Vec<String>) {
    if ctx.methods.enabled_count() == 0 {
        missing.push("需先开启至少 1 个业务子接口".to_string());
    }
    if front.name == FrontName::SignGateway {
        if !has_app_for_enabled_methods(ctx.apps, ctx.methods) {
            missing.push("需先配置至少 1 个授权了已启用子接口的应用凭据".to_string());
        }
        return;
    }
    if front.auth.is_empty() {
        missing.push("未配置允许的鉴权方式".to_string());
        return;
    }
    for m in &front.auth {
        if !front.name.supported_auth().contains(m) {
            missing.push(format!(
                "接入面 {} 不支持鉴权方式 {}",
                front.name.as_str(),
                m.as_str()
            ));
        }
    }
    if !front.auth.contains(&AuthMethod::None)
        && !has_app_for_enabled_methods(ctx.apps, ctx.methods)
    {
        missing.push("需先配置至少 1 个授权了已启用子接口的应用凭据".to_string());
    }
}

fn check_envelope_front(front: &FrontRecord, ctx: &FrontEvalInput<'_>, missing: &mut Vec<String>) {
    if ctx.profiles.is_empty() {
        missing.push("需先配置至少 1 条信封画像".to_string());
        return;
    }
    if front.name.is_token_envelope() {
        let has_issue = ctx.profiles.iter().any(|p| p.kind == "issue");
        let has_call = ctx.profiles.iter().any(|p| p.kind == "call");
        if !has_issue || !has_call {
            missing.push("换票信封面须同时配置 issue 与 call 画像".to_string());
        }
    }
    for err in ctx.envelope_errors {
        missing.push(err.clone());
    }
}

fn has_app_for_enabled_methods(apps: &[AppRecord], methods: MethodsState) -> bool {
    apps.iter()
        .any(|app| app.methods.iter().any(|m| methods.is_enabled(m)))
}

/// Validates that deleting `target` image does not break any currently enabled image method.
///
/// # Errors
/// Returns [`AppError::Conflict`] if an active image interface relies on `target`.
pub fn guard_delete_image(
    target: &ImageMeta,
    all_images: &[ImageMeta],
    methods: MethodsState,
) -> Result<(), AppError> {
    let counts = SceneCounts::from_images(all_images);
    let mut blocking = Vec::new();
    if methods.imgdef_compare && counts.total() <= 1 {
        blocking.push("x_upt.imgdef.compare 启用中，至少须保留 1 张基准图片".to_string());
    }
    if methods.photos_render {
        let scene_left = match target.scene {
            PhotoScene::ArticleCover => counts.article_cover,
            PhotoScene::Avatar => counts.avatar,
            PhotoScene::Banner => counts.banner,
        };
        if scene_left <= 1 {
            blocking.push(format!(
                "x_upt.photos.render 启用中，场景 {} 至少须保留 1 张图片",
                target.scene.as_str()
            ));
        }
    }
    if blocking.is_empty() {
        Ok(())
    } else {
        Err(AppError::in_use(
            "当前图片被已开启的图片接口依赖，请先停用对应图片接口",
            blocking,
        ))
    }
}

/// Validates that deleting a post does not leave an active `x_upt.posts.search` empty.
///
/// # Errors
/// Returns [`AppError::Conflict`] if `posts.search` is enabled and `posts_count <= 1`.
pub fn guard_delete_post(posts_count: usize, methods: MethodsState) -> Result<(), AppError> {
    if methods.posts_search && posts_count <= 1 {
        return Err(AppError::in_use(
            "x_upt.posts.search 启用中，禁止删除最后 1 条帖子",
            vec!["x_upt.posts.search".to_string()],
        ));
    }
    Ok(())
}

/// Validates that disabling a business method does not strand an active core front with 0 methods.
///
/// # Errors
/// Returns [`AppError::Conflict`] if active core fronts would lose their last enabled method.
pub fn guard_disable_method(
    method: BusinessMethod,
    current: MethodsState,
    fronts: &[FrontRecord],
    apps: &[AppRecord],
) -> Result<(), AppError> {
    let mut next = current;
    next.set(method, false);
    let active_core: Vec<&FrontRecord> = fronts
        .iter()
        .filter(|f| f.enabled && !f.name.is_envelope())
        .collect();
    if active_core.is_empty() {
        return Ok(());
    }
    if next.enabled_count() == 0 {
        let names = active_core
            .iter()
            .map(|f| f.name.as_str().to_string())
            .collect();
        return Err(AppError::in_use(
            "仍有核心接入面处于启用状态，无法关闭最后 1 个业务子接口",
            names,
        ));
    }
    let mut broken = Vec::new();
    for f in active_core {
        let needs_app = f.name == FrontName::SignGateway || !f.auth.contains(&AuthMethod::None);
        if needs_app && !has_app_for_enabled_methods(apps, next) {
            broken.push(f.name.as_str().to_string());
        }
    }
    if broken.is_empty() {
        Ok(())
    } else {
        Err(AppError::in_use(
            "关闭该子接口后，已启用接入面将没有已授权的可用应用凭据",
            broken,
        ))
    }
}
