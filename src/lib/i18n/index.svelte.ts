/** Interface translations.
 *
 * Every locale is imported statically rather than fetched on demand. They are
 * small, they are needed before the first paint of either window, and a window
 * that has to wait for its own words is a window that flashes English first.
 *
 * The contract is `en.json`: it is the only file written by hand against the
 * UI, and every other locale mirrors its keys. A key missing from a translation
 * falls back to English rather than showing the key itself, so a half-finished
 * locale degrades into a mixed-language UI instead of a broken one.
 */

import ar from './locales/ar.json'
import de from './locales/de.json'
import en from './locales/en.json'
import es from './locales/es.json'
import fr from './locales/fr.json'
import it from './locales/it.json'
import ja from './locales/ja.json'
import pt from './locales/pt.json'
import ru from './locales/ru.json'
import zh from './locales/zh.json'

export type LocaleCode = 'en' | 'ru' | 'de' | 'es' | 'pt' | 'it' | 'zh' | 'ja' | 'fr' | 'ar'

/** The stored value, which is a locale or the instruction to follow Windows. */
export type LanguageSetting = 'system' | LocaleCode

type Dict = Record<string, string>

const DICTS: Record<LocaleCode, Dict> = { en, ru, de, es, pt, it, zh, ja, fr, ar }

/** Native names, because a language list that names languages in a language
 *  you cannot read is no use to the person looking for theirs. */
export const LANGUAGE_NAMES: Record<LocaleCode, string> = {
  en: 'English',
  ru: 'Русский',
  de: 'Deutsch',
  es: 'Español',
  pt: 'Português',
  it: 'Italiano',
  zh: '中文',
  ja: '日本語',
  fr: 'Français',
  ar: 'العربية',
}

/** The order the picker lists them in: English first as the source language,
 *  then the rest by their English names, which is at least a stable rule. */
export const LOCALE_ORDER: LocaleCode[] = ['en', 'ar', 'zh', 'fr', 'de', 'it', 'ja', 'pt', 'ru', 'es']

/** Written right to left. Only Arabic among these, but naming the property
 *  rather than testing for `'ar'` keeps the next one from being a hunt. */
const RTL: ReadonlySet<LocaleCode> = new Set<LocaleCode>(['ar'])

/** What Windows says the user's language is, narrowed to one we have.
 *
 * Tags come in the `xx` or `xx-YY` form. Only the primary subtag is matched: a
 * user set to `pt-BR` wants Portuguese, and refusing them because the file is
 * not named `pt-BR.json` would be pedantry. */
function systemLocale(): LocaleCode {
  const fallback = typeof navigator !== 'undefined'
    ? [navigator.language, ...(navigator.languages ?? [])]
    : []
  for (const tag of [...systemTags, ...fallback]) {
    const primary = (tag ?? '').toLowerCase().split('-')[0] ?? ''
    if (primary in DICTS) return primary as LocaleCode
  }
  return 'en'
}

let setting = $state<LanguageSetting>('system')
let active = $state<LocaleCode>('en')

/** The OS display languages, as the Rust side reported them. Empty until the
 *  first `setSystemLanguages`, which is why `navigator` is still consulted. */
let systemTags = $state<readonly string[]>([])

/** Hands the i18n layer the languages Windows is actually set to, and re-runs
 *  the resolution if `"system"` is the current setting.
 *
 * This has to come from the OS. `navigator.language` inside WebView2 reports
 * the language the runtime was launched with — `en-US` on every machine —
 * so a user running Windows in Russian was shown an English UI by `"system"`. */
export function setSystemLanguages(tags: readonly string[]): void {
  systemTags = [...tags]
  if (setting === 'system') setLanguage('system')
}

/** The locale actually in use, with `"system"` already resolved. */
export function locale(): LocaleCode {
  return active
}

/** The stored setting, which may still be `"system"`. */
export function languageSetting(): LanguageSetting {
  return setting
}

export function isRtl(): boolean {
  return RTL.has(active)
}

/** Points the UI at a language. Safe to call with anything: an unknown value
 *  falls back the same way a missing key does. */
export function setLanguage(value: string | undefined | null): void {
  const next: LanguageSetting = value === 'system' || (value !== null && value !== undefined && value in DICTS)
    ? (value as LanguageSetting)
    : 'system'
  setting = next
  active = next === 'system' ? systemLocale() : next

  // The document has to be told too: the direction drives every logical CSS
  // property in the sheets, and `lang` is what picks the right font fallbacks
  // and hyphenation for CJK and Arabic.
  if (typeof document !== 'undefined') {
    document.documentElement.lang = active
    document.documentElement.dir = RTL.has(active) ? 'rtl' : 'ltr'
  }
}

/** One translated string.
 *
 * `params` fills `{name}` placeholders. Reading `active` here is what makes
 * every call site reactive: change the language and each one re-runs.
 */
export function t(key: string, params?: Record<string, string | number>): string {
  const code = active
  const raw = DICTS[code]?.[key] ?? DICTS.en[key] ?? key
  if (!params) return raw
  return raw.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole,
  )
}
