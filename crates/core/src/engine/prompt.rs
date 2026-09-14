use crate::lang::{self, Lang};

pub const DEFAULT_SYSTEM_PROMPT: &str = "You are a professional translator. \
Translate the user's text accurately and naturally. \
Output ONLY the translation, without explanations, quotes, or notes.";

pub const DEFAULT_USER_TEMPLATE: &str =
    "Translate the following {source} text to {target}:\n\n{text}";

pub fn render_user_prompt(template: &str, text: &str, from: Lang, to: Lang) -> String {
    template
        .replace("{source}", lang::display(from))
        .replace("{target}", lang::display(to))
        .replace("{text}", text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Lang;

    #[test]
    fn default_template_renders() {
        let out = render_user_prompt(DEFAULT_USER_TEMPLATE, "hello", Lang::En, Lang::Zh);
        assert!(out.contains("hello"), "{out}");
        assert!(out.contains("English"), "{out}");
        assert!(out.contains("Chinese (Simplified)"), "{out}");
    }

    #[test]
    fn custom_template() {
        let out = render_user_prompt(
            "把这句{source}翻成{target}：{text}",
            "hi",
            Lang::En,
            Lang::Zh,
        );
        assert_eq!(out, "把这句English翻成Chinese (Simplified)：hi");
    }

    #[test]
    fn unknown_placeholders_left_as_is() {
        let out = render_user_prompt("{text} {nope}", "x", Lang::Zh, Lang::En);
        assert_eq!(out, "x {nope}");
    }
}
