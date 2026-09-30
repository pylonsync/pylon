#nullable enable
using UnityEngine;

namespace Pylon.Unity
{
    /// <summary>
    /// Converts a <c>[Serializable]</c> class or struct through Unity's
    /// <see cref="JsonUtility"/>, which works under IL2CPP and on any thread.
    /// JsonUtility reads and writes public fields and <c>[SerializeField]</c>
    /// fields; it does not support dictionaries or a top-level array.
    /// </summary>
    public sealed class JsonUtilityConverter<T> : IPylonConverter<T>
    {
        public static readonly JsonUtilityConverter<T> Instance = new JsonUtilityConverter<T>();

        public PylonValue ToValue(T value) => PylonValue.Parse(JsonUtility.ToJson(value));

        public T FromValue(PylonValue value) => JsonUtility.FromJson<T>(value.ToJson());
    }
}
