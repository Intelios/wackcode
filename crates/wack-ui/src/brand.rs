//! Model provider brand marks: resolving a model or connection to its maker's logo, and drawing
//! the compiled-in SVG.
//!
//! Logos are ported from Godot-Bench's brand set and compiled into the binary through
//! `crate::icon::Assets`:
//! - Full-colour brand SVGs render via `gpui::img` (full-colour `resvg` rasterisation at 2×) and
//!   desaturate via `.grayscale(true)` in the `Mono` theme.
//! - Pure text-ink marks (`Grok`, `Cognition`) render via `gpui::svg` tinted with `theme.text`,
//!   so they follow every theme, light/dark, `Mono` and backdrop contrast lifting.
//! - Dual-tone marks (`Kimi`, `Yi`) ship light and dark variants so their neutral stroke contrasts
//!   in both appearances while keeping their accent dot (grayscale in `Mono`).
//! - Unrecognised models return `None` and draw nothing.

use std::sync::LazyLock;

use gpui::{AnyElement, ObjectFit, Pixels, Styled, StyledImage, img, prelude::*, svg};
use regex::Regex;
use wack_core::{Connection, Model, Resolved, ThemeName};

use crate::theme::{Appearance, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Brand {
    Claude,
    Gemini,
    Gemma,
    Openai,
    Glm,
    Deepseek,
    Qwen,
    Grok,
    Kimi,
    Meta,
    Cognition,
    Mistral,
    Microsoft,
    Cohere,
    Minimax,
    Xiaomi,
    Doubao,
    Yi,
    Stepfun,
    Nvidia,
    Nova,
    Perplexity,
    Hunyuan,
    Ernie,
    Huggingface,
}

impl Brand {
    /// Every supported brand, in display order (for tests and the specimen gallery scene).
    pub const ALL: [Brand; 25] = [
        Brand::Claude,
        Brand::Gemini,
        Brand::Gemma,
        Brand::Openai,
        Brand::Glm,
        Brand::Deepseek,
        Brand::Qwen,
        Brand::Grok,
        Brand::Kimi,
        Brand::Meta,
        Brand::Cognition,
        Brand::Mistral,
        Brand::Microsoft,
        Brand::Cohere,
        Brand::Minimax,
        Brand::Xiaomi,
        Brand::Doubao,
        Brand::Yi,
        Brand::Stepfun,
        Brand::Nvidia,
        Brand::Nova,
        Brand::Perplexity,
        Brand::Hunyuan,
        Brand::Ernie,
        Brand::Huggingface,
    ];

    /// A readable name for the brand (used in scenes and diagnostics).
    pub fn name(self) -> &'static str {
        match self {
            Brand::Claude => "Claude",
            Brand::Gemini => "Gemini",
            Brand::Gemma => "Gemma",
            Brand::Openai => "OpenAI",
            Brand::Glm => "GLM",
            Brand::Deepseek => "DeepSeek",
            Brand::Qwen => "Qwen",
            Brand::Grok => "Grok",
            Brand::Kimi => "Kimi",
            Brand::Meta => "Meta",
            Brand::Cognition => "Cognition",
            Brand::Mistral => "Mistral",
            Brand::Microsoft => "Microsoft",
            Brand::Cohere => "Cohere",
            Brand::Minimax => "MiniMax",
            Brand::Xiaomi => "Xiaomi",
            Brand::Doubao => "Doubao",
            Brand::Yi => "Yi",
            Brand::Stepfun => "StepFun",
            Brand::Nvidia => "NVIDIA",
            Brand::Nova => "Nova",
            Brand::Perplexity => "Perplexity",
            Brand::Hunyuan => "Hunyuan",
            Brand::Ernie => "ERNIE",
            Brand::Huggingface => "Hugging Face",
        }
    }

    /// Asset path for `appearance`.
    pub fn path(self, appearance: Appearance) -> &'static str {
        match self {
            Brand::Claude => "logos/claude.svg",
            Brand::Gemini => "logos/gemini.svg",
            Brand::Gemma => "logos/gemma.svg",
            Brand::Openai => "logos/openai.svg",
            Brand::Glm => "logos/glm.svg",
            Brand::Deepseek => "logos/deepseek.svg",
            Brand::Qwen => "logos/qwen.svg",
            Brand::Grok => "logos/grok.svg",
            Brand::Kimi => match appearance {
                Appearance::Light => "logos/kimi-light.svg",
                Appearance::Dark => "logos/kimi-dark.svg",
            },
            Brand::Meta => "logos/meta.svg",
            Brand::Cognition => "logos/cognition.svg",
            Brand::Mistral => "logos/mistral.svg",
            Brand::Microsoft => "logos/microsoft.svg",
            Brand::Cohere => "logos/cohere.svg",
            Brand::Minimax => "logos/minimax.svg",
            Brand::Xiaomi => "logos/xiaomi.svg",
            Brand::Doubao => "logos/doubao.svg",
            Brand::Yi => match appearance {
                Appearance::Light => "logos/yi-light.svg",
                Appearance::Dark => "logos/yi-dark.svg",
            },
            Brand::Stepfun => "logos/stepfun.svg",
            Brand::Nvidia => "logos/nvidia.svg",
            Brand::Nova => "logos/nova.svg",
            Brand::Perplexity => "logos/perplexity.svg",
            Brand::Hunyuan => "logos/hunyuan.svg",
            Brand::Ernie => "logos/ernie.svg",
            Brand::Huggingface => "logos/huggingface.svg",
        }
    }

    /// Draws the brand's logo at `size` points square, bare (no background chip).
    pub fn view(self, size: Pixels, theme: &Theme) -> AnyElement {
        match self {
            Brand::Grok | Brand::Cognition => svg()
                .path(self.path(theme.appearance))
                .flex_none()
                .size(size)
                .text_color(theme.text)
                .into_any_element(),
            _ => {
                let mono = theme.input.theme == ThemeName::Mono;
                img(self.path(theme.appearance))
                    .flex_none()
                    .size(size)
                    .object_fit(ObjectFit::Contain)
                    .grayscale(mono)
                    .into_any_element()
            }
        }
    }
}

/// Every compiled-in logo asset, served by [`crate::icon::Assets`].
pub(crate) const FILES: &[(&str, &[u8])] = &[
    ("logos/claude.svg", include_bytes!("../assets/logos/claude.svg")),
    ("logos/gemini.svg", include_bytes!("../assets/logos/gemini.svg")),
    ("logos/gemma.svg", include_bytes!("../assets/logos/gemma.svg")),
    ("logos/openai.svg", include_bytes!("../assets/logos/openai.svg")),
    ("logos/glm.svg", include_bytes!("../assets/logos/glm.svg")),
    ("logos/deepseek.svg", include_bytes!("../assets/logos/deepseek.svg")),
    ("logos/qwen.svg", include_bytes!("../assets/logos/qwen.svg")),
    ("logos/grok.svg", include_bytes!("../assets/logos/grok.svg")),
    ("logos/kimi-light.svg", include_bytes!("../assets/logos/kimi-light.svg")),
    ("logos/kimi-dark.svg", include_bytes!("../assets/logos/kimi-dark.svg")),
    ("logos/meta.svg", include_bytes!("../assets/logos/meta.svg")),
    ("logos/cognition.svg", include_bytes!("../assets/logos/cognition.svg")),
    ("logos/mistral.svg", include_bytes!("../assets/logos/mistral.svg")),
    ("logos/microsoft.svg", include_bytes!("../assets/logos/microsoft.svg")),
    ("logos/cohere.svg", include_bytes!("../assets/logos/cohere.svg")),
    ("logos/minimax.svg", include_bytes!("../assets/logos/minimax.svg")),
    ("logos/xiaomi.svg", include_bytes!("../assets/logos/xiaomi.svg")),
    ("logos/doubao.svg", include_bytes!("../assets/logos/doubao.svg")),
    ("logos/yi-light.svg", include_bytes!("../assets/logos/yi-light.svg")),
    ("logos/yi-dark.svg", include_bytes!("../assets/logos/yi-dark.svg")),
    ("logos/stepfun.svg", include_bytes!("../assets/logos/stepfun.svg")),
    ("logos/nvidia.svg", include_bytes!("../assets/logos/nvidia.svg")),
    ("logos/nova.svg", include_bytes!("../assets/logos/nova.svg")),
    ("logos/perplexity.svg", include_bytes!("../assets/logos/perplexity.svg")),
    ("logos/hunyuan.svg", include_bytes!("../assets/logos/hunyuan.svg")),
    ("logos/ernie.svg", include_bytes!("../assets/logos/ernie.svg")),
    ("logos/huggingface.svg", include_bytes!("../assets/logos/huggingface.svg")),
];

struct Rule {
    brand: Brand,
    pattern: Regex,
}

fn rule(brand: Brand, pattern: &str) -> Rule {
    Rule { brand, pattern: Regex::new(pattern).expect("valid brand regex") }
}

/// Pass 1: Model-family keywords (matched by earliest occurrence in the string).
/// Capture group 1 always frames the matched keyword so start indices compare fairly across rules.
static MODEL_FAMILY_RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(Brand::Claude, r"(?i)(?:^|[^a-z0-9])(claude|sonnet|opus|haiku)(?:$|[^a-z])"),
        rule(Brand::Gemma, r"(?i)(?:^|[^a-z0-9])(gemma|codegemma|paligemma)(?:$|[^a-z])"),
        rule(Brand::Gemini, r"(?i)(?:^|[^a-z0-9])(gemini|bard|palm)(?:$|[^a-z])"),
        rule(Brand::Glm, r"(?i)(?:^|[^a-z0-9])(glm|chatglm|codegeex|cogview)(?:$|[^a-z])"),
        rule(Brand::Deepseek, r"(?i)(?:^|[^a-z0-9])(deepseek)(?:$|[^a-z])"),
        rule(Brand::Qwen, r"(?i)(?:^|[^a-z0-9])(qwen|qwq|qvq|tongyi)(?:$|[^a-z])|(千问)"),
        rule(Brand::Openai, r"(?i)(?:^|[^a-z0-9])(chatgpt|gpt|codex|dall-e|whisper)(?:$|[^a-z])|(?:^|[^a-z0-9])(o[134](?:-[a-z0-9]+)?)(?:$|[^a-z0-9])"),
        rule(Brand::Grok, r"(?i)(?:^|[^a-z0-9])(grok)(?:$|[^a-z])"),
        rule(Brand::Kimi, r"(?i)(?:^|[^a-z0-9])(kimi)(?:$|[^a-z])"),
        rule(Brand::Meta, r"(?i)(?:^|[^a-z0-9])(llama|codellama)(?:$|[^a-z])"),
        rule(Brand::Cognition, r"(?i)(?:^|[^a-z0-9])(swe[- ]?\d[a-z0-9._-]*)(?:$|[^a-z0-9])"),
        rule(Brand::Mistral, r"(?i)(?:^|[^a-z0-9])(mistral|mixtral|codestral|pixtral|ministral|devstral|mathstral|magistral)(?:$|[^a-z])"),
        rule(Brand::Microsoft, r"(?i)(?:^|[^a-z0-9])(phi|wizardlm|copilot)(?:$|[^a-z])"),
        rule(Brand::Cohere, r"(?i)(?:^|[^a-z0-9])(command-r|command-a|aya)(?:$|[^a-z])"),
        rule(Brand::Minimax, r"(?i)(?:^|[^a-z0-9])(minimax|abab|hailuo)(?:$|[^a-z])"),
        rule(Brand::Xiaomi, r"(?i)(?:^|[^a-z0-9])(mimo)(?:$|[^a-z])"),
        rule(Brand::Doubao, r"(?i)(?:^|[^a-z0-9])(doubao|seed-coder|skylark)(?:$|[^a-z])|(豆包)"),
        rule(Brand::Yi, r"(?i)(?:^|[^a-z0-9])(yi)(?:$|[^a-z])"),
        rule(Brand::Stepfun, r"(?i)(?:^|[^a-z0-9])(step-[123])(?:$|[^a-z0-9])"),
        rule(Brand::Nvidia, r"(?i)(?:^|[^a-z0-9])(nemotron)(?:$|[^a-z])"),
        rule(Brand::Nova, r"(?i)(?:^|[^a-z0-9])(nova|titan|bedrock)(?:$|[^a-z])"),
        rule(Brand::Perplexity, r"(?i)(?:^|[^a-z0-9])(sonar)(?:$|[^a-z])"),
        rule(Brand::Hunyuan, r"(?i)(?:^|[^a-z0-9])(hunyuan)(?:$|[^a-z])|(混元)"),
        rule(Brand::Ernie, r"(?i)(?:^|[^a-z0-9])(ernie|wenxin)(?:$|[^a-z])|(文心)"),
        rule(Brand::Huggingface, r"(?i)(?:^|[^a-z0-9])(smollm|starcoder|zephyr)(?:$|[^a-z])"),
    ]
});

/// Pass 2: Company / vendor names and common API slug prefixes (matched when no model-family keyword matched).
static VENDOR_RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(Brand::Claude, r"(?i)(?:^|[^a-z0-9])(anthropic)(?:$|[^a-z0-9])"),
        rule(Brand::Gemini, r"(?i)(?:^|[^a-z0-9])(google|deepmind)(?:$|[^a-z0-9])"),
        rule(Brand::Glm, r"(?i)(?:^|[^a-z0-9])(zhipu|zhipuai|z[.-]ai)(?:$|[^a-z0-9])|(智谱)"),
        rule(Brand::Deepseek, r"(深度求索)"),
        rule(Brand::Qwen, r"(?i)(?:^|[^a-z0-9])(alibaba|aliyun)(?:$|[^a-z0-9])|(通义)"),
        rule(Brand::Openai, r"(?i)(?:^|[^a-z0-9])(openai)(?:$|[^a-z0-9])"),
        rule(Brand::Grok, r"(?i)(?:^|[^a-z0-9])(xai|x[.-]ai)(?:$|[^a-z0-9])"),
        rule(Brand::Kimi, r"(?i)(?:^|[^a-z0-9])(moonshot|moonshotai)(?:$|[^a-z0-9])|(月之暗面)"),
        rule(Brand::Meta, r"(?i)(?:^|[^a-z0-9])(meta)(?:$|[^a-z0-9])"),
        rule(Brand::Cognition, r"(?i)(?:^|[^a-z0-9])(cognition)(?:$|[^a-z0-9])"),
        rule(Brand::Mistral, r"(?i)(?:^|[^a-z0-9])(mistralai)(?:$|[^a-z0-9])"),
        rule(Brand::Microsoft, r"(?i)(?:^|[^a-z0-9])(microsoft|azure)(?:$|[^a-z0-9])"),
        rule(Brand::Cohere, r"(?i)(?:^|[^a-z0-9])(cohere)(?:$|[^a-z0-9])"),
        rule(Brand::Minimax, r"(?i)(?:^|[^a-z0-9])(minimaxi)(?:$|[^a-z0-9])"),
        rule(Brand::Xiaomi, r"(?i)(?:^|[^a-z0-9])(xiaomi)(?:$|[^a-z0-9])"),
        rule(Brand::Doubao, r"(?i)(?:^|[^a-z0-9])(bytedance)(?:$|[^a-z0-9])|(字节)"),
        rule(Brand::Yi, r"(?i)(?:^|[^a-z0-9])(zeroone|01[.-]ai)(?:$|[^a-z0-9])|(零一万物)"),
        rule(Brand::Stepfun, r"(?i)(?:^|[^a-z0-9])(stepfun)(?:$|[^a-z0-9])|(阶跃星辰)"),
        rule(Brand::Nvidia, r"(?i)(?:^|[^a-z0-9])(nvidia)(?:$|[^a-z0-9])"),
        rule(Brand::Nova, r"(?i)(?:^|[^a-z0-9])(amazon|aws)(?:$|[^a-z0-9])"),
        rule(Brand::Perplexity, r"(?i)(?:^|[^a-z0-9])(perplexity)(?:$|[^a-z0-9])"),
        rule(Brand::Hunyuan, r"(?i)(?:^|[^a-z0-9])(tencent)(?:$|[^a-z0-9])"),
        rule(Brand::Ernie, r"(?i)(?:^|[^a-z0-9])(baidu)(?:$|[^a-z0-9])"),
        rule(Brand::Huggingface, r"(?i)(?:^|[^a-z0-9])(huggingface|hugging face|huggingfaceh4)(?:$|[^a-z0-9])"),
    ]
});

fn match_earliest(input: &str, rules: &[Rule]) -> Option<Brand> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut best: Option<(usize, Brand)> = None;
    for rule in rules {
        let Some(captures) = rule.pattern.captures(trimmed) else {
            continue;
        };
        let start = captures.iter().skip(1).flatten().map(|m| m.start()).min()?;
        if best.is_none_or(|(best_start, _)| start < best_start) {
            best = Some((start, rule.brand));
        }
    }
    best.map(|(_, brand)| brand)
}

fn match_text(input: &str) -> Option<Brand> {
    match_earliest(input, &MODEL_FAMILY_RULES).or_else(|| match_earliest(input, &VENDOR_RULES))
}

/// Direct single-vendor API host fallback. Multi-maker gateways (OpenRouter, Groq, Together,
/// Vercel, localhost, etc.) return `None` so custom models on a gateway never wear a wrong logo.
fn host_brand(base_url: &str) -> Option<Brand> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed.split_once("://").map_or(trimmed, |(_, rest)| rest);
    let authority = without_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = host_port.split(':').next().unwrap_or("").to_ascii_lowercase();
    match host.as_str() {
        "api.anthropic.com" => Some(Brand::Claude),
        "api.openai.com" => Some(Brand::Openai),
        "generativelanguage.googleapis.com" => Some(Brand::Gemini),
        "api.deepseek.com" => Some(Brand::Deepseek),
        "api.mistral.ai" | "codestral.mistral.ai" => Some(Brand::Mistral),
        "api.x.ai" => Some(Brand::Grok),
        "api.z.ai" | "open.bigmodel.cn" => Some(Brand::Glm),
        "api.moonshot.cn" | "api.moonshot.ai" => Some(Brand::Kimi),
        "dashscope.aliyuncs.com" | "dashscope-intl.aliyuncs.com" => Some(Brand::Qwen),
        "api.cohere.com" | "api.cohere.ai" => Some(Brand::Cohere),
        "api.perplexity.ai" => Some(Brand::Perplexity),
        "api.minimax.chat" | "api.minimaxi.chat" | "api.minimax.io" => Some(Brand::Minimax),
        "api.stepfun.com" => Some(Brand::Stepfun),
        "api.lingyiwanwu.com" => Some(Brand::Yi),
        _ => None,
    }
}

/// Resolves a model and its connection to a known model-provider logo, if any.
///
/// Priority:
/// 1. Model-family keywords in `model_name`, then `model_id` (earliest occurrence wins).
/// 2. Vendor / maker keywords in `model_name`, then `model_id`.
/// 3. Direct single-vendor API host in `base_url`.
/// 4. Connection name (family, then vendor).
pub fn resolve(model_id: &str, model_name: Option<&str>, connection_name: &str, base_url: &str) -> Option<Brand> {
    let name = model_name.map(str::trim).filter(|name| !name.is_empty());
    if let Some(brand) = name.and_then(|name| match_earliest(name, &MODEL_FAMILY_RULES)) {
        return Some(brand);
    }
    if let Some(brand) = match_earliest(model_id, &MODEL_FAMILY_RULES) {
        return Some(brand);
    }
    if let Some(brand) = name.and_then(|name| match_earliest(name, &VENDOR_RULES)) {
        return Some(brand);
    }
    if let Some(brand) = match_earliest(model_id, &VENDOR_RULES) {
        return Some(brand);
    }
    host_brand(base_url).or_else(|| match_text(connection_name))
}

/// Resolves a saved model and its connection.
pub fn resolve_for(model: &Model, connection: &Connection) -> Option<Brand> {
    resolve(&model.id, model.name.as_deref(), &connection.name, &connection.base_url)
}

/// Resolves a chat's resolved model and connection.
pub fn resolve_resolved(resolved: &Resolved) -> Option<Brand> {
    resolve_for(&resolved.model, &resolved.connection)
}

#[cfg(test)]
mod tests {
    use gpui::AssetSource;

    use super::*;
    use crate::icon::Assets;

    #[test]
    fn resolves_godot_bench_cases() {
        let cases: &[(&str, &str, Brand)] = &[
            ("GLM 5.3", "", Brand::Glm),
            ("GLM 5.3", "Other", Brand::Glm),
            ("Gemini 3 Flash", "", Brand::Gemini),
            ("Gemini 3.1 Pro", "Google", Brand::Gemini),
            ("Gemma 3 27B", "Google", Brand::Gemma),
            ("Claude Opus 5", "Anthropic", Brand::Claude),
            ("Claude Sonnet 5.5", "", Brand::Claude),
            ("GPT-5.6", "OpenAI", Brand::Openai),
            ("o3-mini", "", Brand::Openai),
            ("DeepSeek V4", "DeepSeek", Brand::Deepseek),
            ("Qwen 3.5 Max", "Qwen", Brand::Qwen),
            ("Grok 5", "xAI", Brand::Grok),
            ("Llama 5 405B", "Meta", Brand::Meta),
            ("SWE-1.5", "Cognition AI", Brand::Cognition),
            ("SWE-1.5 Pro", "", Brand::Cognition),
            ("Custom Fine-Tune", "Cognition AI", Brand::Cognition),
            ("MiMo-VL-7B", "Xiaomi", Brand::Xiaomi),
            ("Custom Fine-Tune", "Xiaomi", Brand::Xiaomi),
            ("Mistral Large 3", "Mistral", Brand::Mistral),
            ("Kimi K3 (智谱 reasoning · 1M context)", "Moonshot", Brand::Kimi),
            ("Google Gemma 3 27B", "Google", Brand::Gemma),
            ("Phi-4", "Microsoft", Brand::Microsoft),
            ("Custom Fine-Tune", "OpenAI", Brand::Openai),
            ("Custom Fine-Tune", "Z.ai", Brand::Glm),
            ("Custom Fine-Tune", "Anthropic", Brand::Claude),
        ];
        for &(model, vendor, expected) in cases {
            assert_eq!(resolve(model, None, vendor, ""), Some(expected), "model={model:?} vendor={vendor:?}");
        }
    }

    #[test]
    fn resolves_wackcode_model_ids_and_hosts() {
        // Model IDs on gateways or generic connections resolve by model family/slug first.
        assert_eq!(resolve("z-ai/glm-5.3", Some("GLM-5.3-Flash"), "Testing", "https://openrouter.ai/api/v1"), Some(Brand::Glm));
        assert_eq!(resolve("qwen/qwen3-coder-30b", None, "Local", "http://localhost:1234/v1"), Some(Brand::Qwen));
        assert_eq!(resolve("qwen3-coder-30b", None, "Local", "http://localhost:1234/v1"), Some(Brand::Qwen));
        assert_eq!(resolve("deepseek-ai/DeepSeek-V4.1-Flash", None, "Testing", "https://openrouter.ai/api/v1"), Some(Brand::Deepseek));
        assert_eq!(resolve("google/gemma-3-27b", None, "OpenRouter", "https://openrouter.ai/api/v1"), Some(Brand::Gemma));
        assert_eq!(resolve("claude_3_5_sonnet", None, "Work", ""), Some(Brand::Claude));
        assert_eq!(resolve("x-ai/grok-4", None, "OpenRouter", "https://openrouter.ai/api/v1"), Some(Brand::Grok));
        assert_eq!(resolve("moonshotai/kimi-k2", None, "OpenRouter", ""), Some(Brand::Kimi));
        assert_eq!(resolve("mistralai/devstral-2", None, "OpenRouter", ""), Some(Brand::Mistral));
        assert_eq!(resolve("01-ai/yi-lightning", None, "OpenRouter", ""), Some(Brand::Yi));
        assert_eq!(resolve("stepfun/step-3", None, "OpenRouter", ""), Some(Brand::Stepfun));
        assert_eq!(resolve("amazon/nova-pro", None, "OpenRouter", ""), Some(Brand::Nova));
        assert_eq!(resolve("perplexity/sonar-pro", None, "OpenRouter", ""), Some(Brand::Perplexity));
        assert_eq!(resolve("cohere/command-a", None, "OpenRouter", ""), Some(Brand::Cohere));
        assert_eq!(resolve("tencent/hunyuan-t1", None, "OpenRouter", ""), Some(Brand::Hunyuan));
        assert_eq!(resolve("baidu/ernie-4.5", None, "OpenRouter", ""), Some(Brand::Ernie));
        assert_eq!(resolve("nvidia/nemotron-70b", None, "OpenRouter", ""), Some(Brand::Nvidia));
        assert_eq!(resolve("bytedance/seed-coder", None, "OpenRouter", ""), Some(Brand::Doubao));
        assert_eq!(resolve("minimax/minimax-m1", None, "OpenRouter", ""), Some(Brand::Minimax));
        assert_eq!(resolve("huggingface/smollm3", None, "OpenRouter", ""), Some(Brand::Huggingface));

        // Direct provider hosts resolve even for unknown preview model IDs.
        assert_eq!(resolve("internal-preview-v2", None, "Work", "https://api.anthropic.com"), Some(Brand::Claude));
        assert_eq!(resolve("internal-preview-v2", None, "Work", "https://api.openai.com/v1"), Some(Brand::Openai));
        assert_eq!(resolve("internal-preview-v2", None, "Work", "https://api.x.ai/v1"), Some(Brand::Grok));

        // Gateways and unknown custom models return None (nothing drawn).
        assert_eq!(resolve("custom-finetune", None, "OpenRouter", "https://openrouter.ai/api/v1"), None);
        assert_eq!(resolve("Lab Test Model", None, "Test Lab", ""), None);
        assert_eq!(resolve("SWE-bench Verified", None, "Test Lab", ""), None);
        assert_eq!(resolve("o300-custom", None, "Local", "http://localhost:1234/v1"), None);
        assert_eq!(resolve("yielding-model", None, "Local", ""), None);
        assert_eq!(resolve("", None, "", ""), None);
    }

    #[test]
    fn every_brand_has_valid_svg_assets() {
        for brand in Brand::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let path = brand.path(appearance);
                let bytes = Assets.load(path).unwrap().unwrap_or_else(|| panic!("missing asset {path}"));
                let text = std::str::from_utf8(&bytes).unwrap_or_else(|_| panic!("non-UTF8 asset {path}"));
                assert!(text.starts_with("<svg "), "{path} should start with <svg");
                assert!(text.contains("viewBox="), "{path} missing viewBox");
                assert!(!text.contains("stopColor") && !text.contains("stopOpacity") && !text.contains("fillRule"), "JSX attribute in {path}");
                assert!(!text.contains("var("), "CSS var in {path}");
            }
        }
    }
}
