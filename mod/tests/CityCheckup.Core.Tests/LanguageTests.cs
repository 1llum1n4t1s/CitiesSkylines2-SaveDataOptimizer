namespace CityCheckup.Core.Tests;

[TestClass]
public sealed class LanguageTests
{
    [TestMethod]
    [DataRow("ja-JP", UiLanguage.Japanese)]
    [DataRow("JA", UiLanguage.Japanese)]
    [DataRow("en-US", UiLanguage.English)]
    [DataRow("de-DE", UiLanguage.English)]
    [DataRow(null, UiLanguage.English)]
    public void FromLocale_UsesJapaneseOnlyForJapanese(string? locale, UiLanguage expected)
        => Assert.AreEqual(expected, UiLanguages.FromLocale(locale));

    [TestMethod]
    public void Findings_AreWrittenInEnglishWhenAsked()
    {
        var pathfind = new PathfindWindow(new Dictionary<PathfindSource, PathfindCounter>
        {
            [PathfindSource.JobSearch] = new(30, 6, 6.0),
            [PathfindSource.Shopping] = new(70, 70, 2.0),
        }, 10, 10);
        var report = new LoadReport { SelectedSpeed = 1, ActualSpeed = 1, Pathfind = pathfind, Language = UiLanguage.English };

        var text = report.Findings().Single().Text;
        Assert.IsTrue(text.Contains("75% of pathfinding work is \"Job search\""), text);
        Assert.IsTrue(text.Contains("Only 20% find a destination"), text);
        // 英語の文に日本語が混ざっていない。
        Assert.IsFalse(text.Any(c => c >= 0x3040 && c <= 0x9FFF), text);
    }

    [TestMethod]
    public void HealthyCity_SaysSoInBothLanguages()
    {
        Assert.AreEqual("No major cause of load was found.", new LoadReport { Language = UiLanguage.English }.Findings().Single().Text);
        Assert.AreEqual("目立った負荷の原因は見つかっていません。", new LoadReport().Findings().Single().Text);
    }
}
