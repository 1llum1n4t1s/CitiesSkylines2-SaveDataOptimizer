using System;

namespace CityCheckup.Core;

/// <summary>パネルと所見の文の言語。日本語以外はすべて英語で表示する。</summary>
public enum UiLanguage
{
    Japanese,
    English,
}

public static class UiLanguages
{
    /// <summary>ゲームの言語設定（LocalizationManager.activeLocaleId、例: ja-JP / en-US）から表示言語を決める。</summary>
    public static UiLanguage FromLocale(string? localeId) =>
        localeId is not null && localeId.StartsWith("ja", StringComparison.OrdinalIgnoreCase) ? UiLanguage.Japanese : UiLanguage.English;

    /// <summary>UI へ渡すキー。</summary>
    public static string Key(UiLanguage language) => language == UiLanguage.Japanese ? "ja" : "en";
}
