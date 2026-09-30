#nullable enable
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace Pylon
{
    /// <summary>A strict RFC 8259 JSON parser into <see cref="PylonValue"/>. No reflection.</summary>
    internal sealed class JsonParser
    {
        const int MaxDepth = 512;

        readonly string _s;
        int _i;
        int _depth;

        JsonParser(string s)
        {
            _s = s;
        }

        public static PylonValue Parse(string json)
        {
            if (json == null) throw new ArgumentNullException(nameof(json));
            var p = new JsonParser(json);
            p.SkipWhitespace();
            var value = p.ReadValue();
            p.SkipWhitespace();
            if (p._i != json.Length) throw p.Error("unexpected text after the value");
            return value;
        }

        PylonException Error(string message) => PylonException.Decoding($"invalid JSON at {_i}: {message}");

        void SkipWhitespace()
        {
            while (_i < _s.Length)
            {
                var c = _s[_i];
                if (c == ' ' || c == '\t' || c == '\n' || c == '\r') _i++;
                else break;
            }
        }

        PylonValue ReadValue()
        {
            if (_i >= _s.Length) throw Error("unexpected end");
            var c = _s[_i];
            switch (c)
            {
                case '{': return ReadObject();
                case '[': return ReadArray();
                case '"': return PylonValue.From(ReadString());
                case 't': Expect("true"); return PylonValue.True;
                case 'f': Expect("false"); return PylonValue.False;
                case 'n': Expect("null"); return PylonValue.Null;
                default:
                    if (c == '-' || (c >= '0' && c <= '9')) return ReadNumber();
                    throw Error($"unexpected character '{c}'");
            }
        }

        void Expect(string word)
        {
            if (string.CompareOrdinal(_s, _i, word, 0, word.Length) != 0) throw Error($"expected {word}");
            _i += word.Length;
        }

        void Enter()
        {
            if (++_depth > MaxDepth) throw Error("nested too deeply");
        }

        PylonValue ReadObject()
        {
            Enter();
            _i++; // {
            var fields = new List<KeyValuePair<string, PylonValue>>();
            SkipWhitespace();
            if (_i < _s.Length && _s[_i] == '}')
            {
                _i++;
                _depth--;
                return PylonValue.OwnObject(fields);
            }
            while (true)
            {
                SkipWhitespace();
                if (_i >= _s.Length || _s[_i] != '"') throw Error("expected a string key");
                var key = ReadString();
                SkipWhitespace();
                if (_i >= _s.Length || _s[_i] != ':') throw Error("expected ':'");
                _i++;
                SkipWhitespace();
                fields.Add(new KeyValuePair<string, PylonValue>(key, ReadValue()));
                SkipWhitespace();
                if (_i >= _s.Length) throw Error("unexpected end in an object");
                if (_s[_i] == ',')
                {
                    _i++;
                    continue;
                }
                if (_s[_i] == '}')
                {
                    _i++;
                    break;
                }
                throw Error("expected ',' or '}'");
            }
            _depth--;
            return PylonValue.OwnObject(fields);
        }

        PylonValue ReadArray()
        {
            Enter();
            _i++; // [
            var items = new List<PylonValue>();
            SkipWhitespace();
            if (_i < _s.Length && _s[_i] == ']')
            {
                _i++;
                _depth--;
                return PylonValue.OwnArray(items.ToArray());
            }
            while (true)
            {
                SkipWhitespace();
                items.Add(ReadValue());
                SkipWhitespace();
                if (_i >= _s.Length) throw Error("unexpected end in an array");
                if (_s[_i] == ',')
                {
                    _i++;
                    continue;
                }
                if (_s[_i] == ']')
                {
                    _i++;
                    break;
                }
                throw Error("expected ',' or ']'");
            }
            _depth--;
            return PylonValue.OwnArray(items.ToArray());
        }

        string ReadString()
        {
            _i++; // opening quote
            StringBuilder? sb = null;
            var start = _i;
            while (true)
            {
                if (_i >= _s.Length) throw Error("unterminated string");
                var c = _s[_i];
                if (c == '"')
                {
                    string result;
                    if (sb == null) result = _s.Substring(start, _i - start);
                    else
                    {
                        sb.Append(_s, start, _i - start);
                        result = sb.ToString();
                    }
                    _i++;
                    return result;
                }
                if (c < 0x20) throw Error("control character in a string");
                if (c != '\\')
                {
                    _i++;
                    continue;
                }
                sb ??= new StringBuilder();
                sb.Append(_s, start, _i - start);
                _i++;
                if (_i >= _s.Length) throw Error("unterminated escape");
                var e = _s[_i++];
                switch (e)
                {
                    case '"': sb.Append('"'); break;
                    case '\\': sb.Append('\\'); break;
                    case '/': sb.Append('/'); break;
                    case 'b': sb.Append('\b'); break;
                    case 'f': sb.Append('\f'); break;
                    case 'n': sb.Append('\n'); break;
                    case 'r': sb.Append('\r'); break;
                    case 't': sb.Append('\t'); break;
                    case 'u':
                        if (_i + 4 > _s.Length) throw Error("short \\u escape");
                        var code = 0;
                        for (var k = 0; k < 4; k++)
                        {
                            var h = _s[_i + k];
                            int d;
                            if (h >= '0' && h <= '9') d = h - '0';
                            else if (h >= 'a' && h <= 'f') d = h - 'a' + 10;
                            else if (h >= 'A' && h <= 'F') d = h - 'A' + 10;
                            else throw Error("bad \\u escape");
                            code = code * 16 + d;
                        }
                        _i += 4;
                        sb.Append((char)code);
                        break;
                    default:
                        throw Error($"bad escape '\\{e}'");
                }
                start = _i;
            }
        }

        PylonValue ReadNumber()
        {
            var start = _i;
            var negative = false;
            if (_s[_i] == '-')
            {
                negative = true;
                _i++;
            }
            if (_i >= _s.Length) throw Error("bad number");
            if (_s[_i] == '0')
            {
                _i++;
            }
            else if (_s[_i] >= '1' && _s[_i] <= '9')
            {
                while (_i < _s.Length && _s[_i] >= '0' && _s[_i] <= '9') _i++;
            }
            else
            {
                throw Error("bad number");
            }
            var isFloat = false;
            if (_i < _s.Length && _s[_i] == '.')
            {
                isFloat = true;
                _i++;
                var digits = _i;
                while (_i < _s.Length && _s[_i] >= '0' && _s[_i] <= '9') _i++;
                if (_i == digits) throw Error("bad fraction");
            }
            if (_i < _s.Length && (_s[_i] == 'e' || _s[_i] == 'E'))
            {
                isFloat = true;
                _i++;
                if (_i < _s.Length && (_s[_i] == '+' || _s[_i] == '-')) _i++;
                var digits = _i;
                while (_i < _s.Length && _s[_i] >= '0' && _s[_i] <= '9') _i++;
                if (_i == digits) throw Error("bad exponent");
            }
            var text = _s.Substring(start, _i - start);
            if (!isFloat)
            {
                if (long.TryParse(text, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var l))
                    return PylonValue.From(l);
                if (!negative && ulong.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out var u))
                    return PylonValue.From(u);
            }
            if (!double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out var d) ||
                double.IsInfinity(d))
            {
                throw Error("number out of range");
            }
            return PylonValue.From(d);
        }
    }

    /// <summary>Writes <see cref="PylonValue"/> as compact JSON.</summary>
    internal static class JsonWriter
    {
        public static string Write(PylonValue value)
        {
            var sb = new StringBuilder();
            Write(sb, value, 0);
            return sb.ToString();
        }

        static void Write(StringBuilder sb, PylonValue v, int depth)
        {
            if (depth > 512) throw PylonException.InvalidArgument("value nested too deeply to write");
            switch (v.Kind)
            {
                case PylonValueKind.Null:
                    sb.Append("null");
                    break;
                case PylonValueKind.Bool:
                    sb.Append(v.AsBool() ? "true" : "false");
                    break;
                case PylonValueKind.Int:
                    sb.Append(v.LongUnsafe.ToString(CultureInfo.InvariantCulture));
                    break;
                case PylonValueKind.UInt:
                    sb.Append(v.ULongUnsafe.ToString(CultureInfo.InvariantCulture));
                    break;
                case PylonValueKind.Float:
                    WriteDouble(sb, v.DoubleUnsafe);
                    break;
                case PylonValueKind.String:
                    WriteString(sb, v.StringUnsafe);
                    break;
                case PylonValueKind.Bytes:
                    WriteString(sb, Convert.ToBase64String(v.BytesUnsafe));
                    break;
                case PylonValueKind.Array:
                    sb.Append('[');
                    var first = true;
                    foreach (var item in v.Items)
                    {
                        if (!first) sb.Append(',');
                        first = false;
                        Write(sb, item, depth + 1);
                    }
                    sb.Append(']');
                    break;
                case PylonValueKind.Object:
                    sb.Append('{');
                    var firstField = true;
                    foreach (var f in v.Fields)
                    {
                        if (!firstField) sb.Append(',');
                        firstField = false;
                        WriteString(sb, f.Key);
                        sb.Append(':');
                        Write(sb, f.Value, depth + 1);
                    }
                    sb.Append('}');
                    break;
            }
        }

        static void WriteDouble(StringBuilder sb, double d)
        {
            if (double.IsNaN(d) || double.IsInfinity(d))
                throw PylonException.InvalidArgument($"JSON has no {d.ToString(CultureInfo.InvariantCulture)}");
            // Whole numbers keep a fraction so they read back as floats.
            var text = d.ToString("R", CultureInfo.InvariantCulture);
            sb.Append(text);
            if (text.IndexOf('.') < 0 && text.IndexOf('E') < 0 && text.IndexOf('e') < 0) sb.Append(".0");
        }

        internal static void WriteString(StringBuilder sb, string s)
        {
            sb.Append('"');
            foreach (var c in s)
            {
                switch (c)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    case '\b': sb.Append("\\b"); break;
                    case '\f': sb.Append("\\f"); break;
                    default:
                        if (c < 0x20 || c == (char)0x2028 || c == (char)0x2029)
                            sb.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
                        else sb.Append(c);
                        break;
                }
            }
            sb.Append('"');
        }
    }
}
