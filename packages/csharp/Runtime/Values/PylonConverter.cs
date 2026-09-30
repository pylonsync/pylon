#nullable enable
using System;

namespace Pylon
{
    /// <summary>
    /// Converts your own type to and from <see cref="PylonValue"/>. The
    /// client's typed overloads take one, so no reflection runs: write the
    /// conversion by hand (it works under IL2CPP), or in Unity use
    /// <c>Pylon.Unity.JsonUtilityConverter&lt;T&gt;</c>.
    /// </summary>
    public interface IPylonConverter<T>
    {
        PylonValue ToValue(T value);
        T FromValue(PylonValue value);
    }

    public static class PylonConverter
    {
        /// <summary>A converter from two functions.</summary>
        public static IPylonConverter<T> Create<T>(Func<T, PylonValue> toValue, Func<PylonValue, T> fromValue) =>
            new FuncConverter<T>(toValue, fromValue);

        /// <summary>The identity converter.</summary>
        public static readonly IPylonConverter<PylonValue> Value =
            new FuncConverter<PylonValue>(v => v ?? PylonValue.Null, v => v);

        sealed class FuncConverter<T> : IPylonConverter<T>
        {
            readonly Func<T, PylonValue> _to;
            readonly Func<PylonValue, T> _from;

            public FuncConverter(Func<T, PylonValue> to, Func<PylonValue, T> from)
            {
                _to = to ?? throw new ArgumentNullException(nameof(to));
                _from = from ?? throw new ArgumentNullException(nameof(from));
            }

            public PylonValue ToValue(T value) => _to(value);
            public T FromValue(PylonValue value) => _from(value);
        }
    }
}
