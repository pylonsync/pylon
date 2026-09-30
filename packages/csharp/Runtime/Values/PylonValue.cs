#nullable enable
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace Pylon
{
    /// <summary>The type of a <see cref="PylonValue"/>.</summary>
    public enum PylonValueKind
    {
        Null,
        Bool,
        /// <summary>A whole number that fits a <c>long</c>.</summary>
        Int,
        /// <summary>A whole number above <c>long.MaxValue</c>.</summary>
        UInt,
        Float,
        String,
        /// <summary>Raw bytes (MessagePack <c>bin</c>). JSON writes them as base64.</summary>
        Bytes,
        Array,
        Object,
    }

    /// <summary>
    /// A JSON or MessagePack value: what function arguments and results,
    /// entity rows, and shard snapshots and inputs are made of.
    ///
    /// Reading a missing key or index returns <see cref="Null"/>, so a
    /// path such as <c>value["players"][0]["x"]</c> never throws; the
    /// <c>As*</c> methods throw <see cref="PylonException"/> when the value
    /// has another type. Values are immutable. Objects keep their key order.
    /// </summary>
    public sealed class PylonValue : IEquatable<PylonValue>
    {
        public static readonly PylonValue Null = new PylonValue(PylonValueKind.Null);
        public static readonly PylonValue True = new PylonValue(PylonValueKind.Bool) { _long = 1 };
        public static readonly PylonValue False = new PylonValue(PylonValueKind.Bool);

        static readonly IReadOnlyList<PylonValue> NoItems = new PylonValue[0];
        static readonly IReadOnlyList<KeyValuePair<string, PylonValue>> NoFields =
            new KeyValuePair<string, PylonValue>[0];

        public PylonValueKind Kind { get; }

        long _long;
        ulong _ulong;
        double _double;
        string? _string;
        byte[]? _bytes;
        PylonValue[]? _items;
        KeyValuePair<string, PylonValue>[]? _fields;
        Dictionary<string, int>? _index;

        PylonValue(PylonValueKind kind)
        {
            Kind = kind;
        }

        // ---- construction ----

        public static PylonValue From(bool value) => value ? True : False;

        public static PylonValue From(long value) => new PylonValue(PylonValueKind.Int) { _long = value };

        public static PylonValue From(ulong value) =>
            value <= long.MaxValue
                ? From((long)value)
                : new PylonValue(PylonValueKind.UInt) { _ulong = value };

        public static PylonValue From(double value) => new PylonValue(PylonValueKind.Float) { _double = value };

        public static PylonValue From(string? value) =>
            value == null ? Null : new PylonValue(PylonValueKind.String) { _string = value };

        /// <summary>Raw bytes. The array is copied.</summary>
        public static PylonValue From(byte[]? value) =>
            value == null ? Null : new PylonValue(PylonValueKind.Bytes) { _bytes = (byte[])value.Clone() };

        internal static PylonValue OwnBytes(byte[] value) => new PylonValue(PylonValueKind.Bytes) { _bytes = value };

        public static PylonValue Array(params PylonValue[] items) => Array((IEnumerable<PylonValue>)items);

        public static PylonValue Array(IEnumerable<PylonValue> items)
        {
            var list = new List<PylonValue>();
            foreach (var item in items) list.Add(item ?? Null);
            return new PylonValue(PylonValueKind.Array) { _items = list.ToArray() };
        }

        internal static PylonValue OwnArray(PylonValue[] items) => new PylonValue(PylonValueKind.Array) { _items = items };

        /// <summary>An object with these fields, in this order. A repeated key keeps its last value.</summary>
        public static PylonValue Object(params (string Key, PylonValue Value)[] fields)
        {
            var list = new List<KeyValuePair<string, PylonValue>>(fields.Length);
            foreach (var (key, value) in fields) list.Add(new KeyValuePair<string, PylonValue>(key, value ?? Null));
            return OwnObject(list);
        }

        public static PylonValue Object(IEnumerable<KeyValuePair<string, PylonValue>> fields)
        {
            var list = new List<KeyValuePair<string, PylonValue>>();
            foreach (var f in fields) list.Add(new KeyValuePair<string, PylonValue>(f.Key, f.Value ?? Null));
            return OwnObject(list);
        }

        internal static PylonValue OwnObject(List<KeyValuePair<string, PylonValue>> fields)
        {
            if (fields.Count > 1)
            {
                // Keep the last value of a repeated key, at its first position.
                var seen = new Dictionary<string, int>(fields.Count, StringComparer.Ordinal);
                var deduped = new List<KeyValuePair<string, PylonValue>>(fields.Count);
                foreach (var f in fields)
                {
                    if (f.Key == null) throw new ArgumentException("an object key is null");
                    if (seen.TryGetValue(f.Key, out var at)) deduped[at] = f;
                    else
                    {
                        seen[f.Key] = deduped.Count;
                        deduped.Add(f);
                    }
                }
                fields = deduped;
            }
            else if (fields.Count == 1 && fields[0].Key == null)
            {
                throw new ArgumentException("an object key is null");
            }
            return new PylonValue(PylonValueKind.Object) { _fields = fields.ToArray() };
        }

        public static implicit operator PylonValue(bool value) => From(value);
        public static implicit operator PylonValue(int value) => From(value);
        public static implicit operator PylonValue(long value) => From(value);
        public static implicit operator PylonValue(uint value) => From((long)value);
        public static implicit operator PylonValue(ulong value) => From(value);
        public static implicit operator PylonValue(float value) => From((double)value);
        public static implicit operator PylonValue(double value) => From(value);
        public static implicit operator PylonValue(string? value) => From(value);
        public static implicit operator PylonValue(byte[]? value) => From(value);

        // ---- reading ----

        public bool IsNull => Kind == PylonValueKind.Null;

        public bool IsNumber =>
            Kind == PylonValueKind.Int || Kind == PylonValueKind.UInt || Kind == PylonValueKind.Float;

        /// <summary>The field <paramref name="key"/>, or <see cref="Null"/> when this is not an object or has no such field.</summary>
        public PylonValue this[string key] => TryGet(key, out var v) ? v : Null;

        /// <summary>The item at <paramref name="index"/>, or <see cref="Null"/> when this is not an array or the index is out of range.</summary>
        public PylonValue this[int index] =>
            _items != null && index >= 0 && index < _items.Length ? _items[index] : Null;

        public bool Has(string key) => TryGet(key, out _);

        public bool TryGet(string key, out PylonValue value)
        {
            value = Null;
            if (_fields == null) return false;
            if (_fields.Length <= 8)
            {
                foreach (var f in _fields)
                {
                    if (string.Equals(f.Key, key, StringComparison.Ordinal))
                    {
                        value = f.Value;
                        return true;
                    }
                }
                return false;
            }
            if (_index == null)
            {
                var index = new Dictionary<string, int>(_fields.Length, StringComparer.Ordinal);
                for (var i = 0; i < _fields.Length; i++) index[_fields[i].Key] = i;
                _index = index;
            }
            if (!_index.TryGetValue(key, out var at)) return false;
            value = _fields[at].Value;
            return true;
        }

        /// <summary>The items of an array; empty for any other value.</summary>
        public IReadOnlyList<PylonValue> Items => _items ?? NoItems;

        /// <summary>The fields of an object, in order; empty for any other value.</summary>
        public IReadOnlyList<KeyValuePair<string, PylonValue>> Fields => _fields ?? NoFields;

        /// <summary>Items of an array or fields of an object; 0 otherwise.</summary>
        public int Count => _items?.Length ?? _fields?.Length ?? 0;

        public bool AsBool()
        {
            if (Kind != PylonValueKind.Bool) throw TypeError("a bool");
            return _long != 0;
        }

        public string AsString()
        {
            if (Kind != PylonValueKind.String) throw TypeError("a string");
            return _string!;
        }

        /// <summary>This string, or <paramref name="fallback"/> when it is not a string.</summary>
        public string? AsStringOr(string? fallback) => Kind == PylonValueKind.String ? _string : fallback;

        /// <summary>A whole number as a <c>long</c>. A float counts when it holds a whole number in range.</summary>
        public long AsLong()
        {
            switch (Kind)
            {
                case PylonValueKind.Int:
                    return _long;
                case PylonValueKind.Float
                    when Math.Floor(_double) == _double && _double >= -9.2233720368547758E18 && _double < 9.2233720368547758E18:
                    return (long)_double;
                default:
                    throw TypeError("a whole number within the range of a long");
            }
        }

        /// <summary>A whole number that is not negative, as a <c>ulong</c>.</summary>
        public ulong AsULong()
        {
            switch (Kind)
            {
                case PylonValueKind.Int when _long >= 0:
                    return (ulong)_long;
                case PylonValueKind.UInt:
                    return _ulong;
                case PylonValueKind.Float
                    when Math.Floor(_double) == _double && _double >= 0 && _double < 1.8446744073709552E19:
                    return (ulong)_double;
                default:
                    throw TypeError("a whole number that is not negative");
            }
        }

        public int AsInt()
        {
            var v = AsLong();
            if (v < int.MinValue || v > int.MaxValue) throw TypeError("a whole number within the range of an int");
            return (int)v;
        }

        public double AsDouble()
        {
            switch (Kind)
            {
                case PylonValueKind.Int:
                    return _long;
                case PylonValueKind.UInt:
                    return _ulong;
                case PylonValueKind.Float:
                    return _double;
                default:
                    throw TypeError("a number");
            }
        }

        public float AsFloat() => (float)AsDouble();

        /// <summary>Raw bytes. A copy; the value does not change.</summary>
        public byte[] AsBytes()
        {
            if (Kind != PylonValueKind.Bytes) throw TypeError("bytes");
            return (byte[])_bytes!.Clone();
        }

        internal byte[] BytesUnsafe => _bytes!;
        internal long LongUnsafe => _long;
        internal ulong ULongUnsafe => _ulong;
        internal double DoubleUnsafe => _double;
        internal string StringUnsafe => _string!;

        PylonException TypeError(string expected) =>
            PylonException.Decoding($"expected {expected}, got {Describe()}");

        string Describe()
        {
            switch (Kind)
            {
                case PylonValueKind.Null: return "null";
                case PylonValueKind.Bool: return _long != 0 ? "true" : "false";
                case PylonValueKind.Int: return _long.ToString(CultureInfo.InvariantCulture);
                case PylonValueKind.UInt: return _ulong.ToString(CultureInfo.InvariantCulture);
                case PylonValueKind.Float: return _double.ToString("R", CultureInfo.InvariantCulture);
                case PylonValueKind.String: return "a string";
                case PylonValueKind.Bytes: return "bytes";
                case PylonValueKind.Array: return "an array";
                default: return "an object";
            }
        }

        // ---- JSON ----

        /// <summary>Parse JSON text. Throws <see cref="PylonException"/> on invalid JSON.</summary>
        public static PylonValue Parse(string json) => JsonParser.Parse(json);

        /// <summary>Parse UTF-8 JSON bytes.</summary>
        public static PylonValue Parse(byte[] utf8Json) => JsonParser.Parse(Encoding.UTF8.GetString(utf8Json));

        /// <summary>This value as compact JSON. Bytes become base64 strings.</summary>
        public string ToJson() => JsonWriter.Write(this);

        public override string ToString() => ToJson();

        // ---- equality ----

        /// <summary>
        /// Structural equality. Numbers compare by value, so <c>1</c> equals
        /// <c>1.0</c>. Object fields compare regardless of order.
        /// </summary>
        public bool Equals(PylonValue? other)
        {
            if (ReferenceEquals(this, other)) return true;
            if (other is null) return false;
            if (IsNumber && other.IsNumber) return NumberEquals(this, other);
            if (Kind != other.Kind) return false;
            switch (Kind)
            {
                case PylonValueKind.Null:
                    return true;
                case PylonValueKind.Bool:
                    return _long == other._long;
                case PylonValueKind.String:
                    return string.Equals(_string, other._string, StringComparison.Ordinal);
                case PylonValueKind.Bytes:
                    return BytesEqual(_bytes!, other._bytes!);
                case PylonValueKind.Array:
                    if (_items!.Length != other._items!.Length) return false;
                    for (var i = 0; i < _items.Length; i++)
                    {
                        if (!_items[i].Equals(other._items[i])) return false;
                    }
                    return true;
                case PylonValueKind.Object:
                    if (_fields!.Length != other._fields!.Length) return false;
                    foreach (var f in _fields)
                    {
                        if (!other.TryGet(f.Key, out var v) || !f.Value.Equals(v)) return false;
                    }
                    return true;
                default:
                    return false;
            }
        }

        static bool NumberEquals(PylonValue a, PylonValue b)
        {
            if (a.Kind == PylonValueKind.Float || b.Kind == PylonValueKind.Float) return a.AsDouble() == b.AsDouble();
            if (a.Kind != b.Kind) return false;
            return a.Kind == PylonValueKind.Int ? a._long == b._long : a._ulong == b._ulong;
        }

        internal static bool BytesEqual(byte[] a, byte[] b)
        {
            if (a.Length != b.Length) return false;
            for (var i = 0; i < a.Length; i++)
            {
                if (a[i] != b[i]) return false;
            }
            return true;
        }

        public override bool Equals(object? obj) => obj is PylonValue v && Equals(v);

        public override int GetHashCode()
        {
            switch (Kind)
            {
                case PylonValueKind.Int:
                case PylonValueKind.UInt:
                case PylonValueKind.Float:
                    return AsDouble().GetHashCode();
                case PylonValueKind.String:
                    return StringComparer.Ordinal.GetHashCode(_string!);
                case PylonValueKind.Bool:
                    return _long.GetHashCode();
                default:
                    return (int)Kind * 397 ^ Count;
            }
        }

        public static bool operator ==(PylonValue? a, PylonValue? b) => a is null ? b is null : a.Equals(b);
        public static bool operator !=(PylonValue? a, PylonValue? b) => !(a == b);
    }
}
