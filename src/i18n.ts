/// Lightweight i18n — returns zh-CN or en string based on browser/system language.
/// Usage: t("启动", "Start") or t("删除中...", "Deleting...")

let cachedLang: string | null = null;

export function isZh(): boolean {
  if (cachedLang === null) {
    cachedLang = navigator.language || "en";
  }
  return cachedLang.startsWith("zh");
}

/// Return zh or en string based on current language
export function t(zh: string, en: string): string {
  return isZh() ? zh : en;
}
