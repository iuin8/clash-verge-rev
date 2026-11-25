import type { TranslationKey } from '@/types/generated/i18n-keys'

type Translate = (key: TranslationKey) => string

const translateCataloguedCode = (
  key: TranslationKey,
  fallback: string,
  t: Translate,
) => {
  const translated = t(key)
  return translated === key ? fallback : translated
}

/** Localizes backend error codes when the payload carries a bare code. */
export const translateOpenP2pErrorText = (value: string, t: Translate) =>
  translateCataloguedCode(`openp2p.errors.${value}` as TranslationKey, value, t)
