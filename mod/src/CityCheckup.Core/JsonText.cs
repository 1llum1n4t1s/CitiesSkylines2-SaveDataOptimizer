using System.Globalization;
using System.Text;

namespace CityCheckup.Core;

/// <summary>
/// UI へ渡す表示データを JSON 文字列として組み立てる小さな書き出し器。
/// </summary>
/// <remarks>
/// ゲームのバインディング（Colossal.UI.Binding）にも JSON 書き出しはあるが、Core はゲームに依存させない方針のため、
/// 表示データは Core で JSON 文字列にし、mod 側は文字列の ValueBinding で渡すだけにする。
/// UI 側は JSON.parse で受け取る。カンマの挿入は呼び出し側が意識しなくて済むよう、ここで管理する。
/// </remarks>
public sealed class JsonText
{
    private readonly StringBuilder m_Text = new();

    // 直前に値を書いたかどうか。次の値の前にカンマが要るかの判定に使う。
    private bool m_NeedComma;

    public JsonText BeginObject() => Open('{');

    public JsonText EndObject() => Close('}');

    public JsonText BeginArray() => Open('[');

    public JsonText EndArray() => Close(']');

    /// <summary>オブジェクトのキーを書く。続けて値を書くこと。</summary>
    public JsonText Key(string name)
    {
        Separate();
        WriteString(name);
        m_Text.Append(':');
        // キーの直後の値の前にはカンマを置かない。
        m_NeedComma = false;
        return this;
    }

    public JsonText Value(string? text)
    {
        Separate();
        if (text is null)
        {
            m_Text.Append("null");
        }
        else
        {
            WriteString(text);
        }
        m_NeedComma = true;
        return this;
    }

    public JsonText Value(long number)
    {
        Separate();
        m_Text.Append(number.ToString(CultureInfo.InvariantCulture));
        m_NeedComma = true;
        return this;
    }

    public JsonText Value(double number)
    {
        Separate();
        // NaN・無限大は JSON に書けないので null にする（UI 側では「不明」と表示する）。
        if (double.IsNaN(number) || double.IsInfinity(number))
        {
            m_Text.Append("null");
        }
        else
        {
            m_Text.Append(number.ToString("R", CultureInfo.InvariantCulture));
        }
        m_NeedComma = true;
        return this;
    }

    public JsonText Value(bool flag)
    {
        Separate();
        m_Text.Append(flag ? "true" : "false");
        m_NeedComma = true;
        return this;
    }

    public override string ToString() => m_Text.ToString();

    private JsonText Open(char bracket)
    {
        Separate();
        m_Text.Append(bracket);
        m_NeedComma = false;
        return this;
    }

    private JsonText Close(char bracket)
    {
        m_Text.Append(bracket);
        m_NeedComma = true;
        return this;
    }

    private void Separate()
    {
        if (m_NeedComma)
        {
            m_Text.Append(',');
        }
    }

    private void WriteString(string text)
    {
        m_Text.Append('"');
        foreach (var c in text)
        {
            switch (c)
            {
                case '"': m_Text.Append("\\\""); break;
                case '\\': m_Text.Append("\\\\"); break;
                case '\n': m_Text.Append("\\n"); break;
                case '\r': m_Text.Append("\\r"); break;
                case '\t': m_Text.Append("\\t"); break;
                default:
                    // 制御文字は \uXXXX にする。それ以外（日本語を含む）はそのまま書く。
                    if (c < 0x20)
                    {
                        m_Text.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
                    }
                    else
                    {
                        m_Text.Append(c);
                    }
                    break;
            }
        }
        m_Text.Append('"');
    }
}
