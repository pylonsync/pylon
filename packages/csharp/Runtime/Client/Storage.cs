#nullable enable
using System;
using System.Collections.Generic;
using System.IO;

namespace Pylon
{
    /// <summary>
    /// A small key-value store for the session token. Get, set, and remove
    /// run on the calling thread and must be safe to call from any thread.
    /// In Unity, <c>Pylon.Unity.PlayerPrefsStorage</c> keeps the token
    /// between runs.
    /// </summary>
    public interface IPylonStorage
    {
        string? Get(string key);
        void Set(string key, string value);
        void Remove(string key);
    }

    /// <summary>Keeps values in memory; they are gone when the process ends.</summary>
    public sealed class MemoryStorage : IPylonStorage
    {
        readonly object _lock = new object();
        readonly Dictionary<string, string> _map = new Dictionary<string, string>(StringComparer.Ordinal);

        public string? Get(string key)
        {
            lock (_lock) return _map.TryGetValue(key, out var v) ? v : null;
        }

        public void Set(string key, string value)
        {
            lock (_lock) _map[key] = value;
        }

        public void Remove(string key)
        {
            lock (_lock) _map.Remove(key);
        }
    }

    /// <summary>
    /// Keeps values in a JSON file, for desktop tools and dedicated servers.
    /// Each write replaces the file.
    /// </summary>
    public sealed class FileStorage : IPylonStorage
    {
        readonly object _lock = new object();
        readonly string _path;
        Dictionary<string, string>? _map;

        public FileStorage(string path)
        {
            _path = path ?? throw new ArgumentNullException(nameof(path));
        }

        Dictionary<string, string> Load()
        {
            if (_map != null) return _map;
            var map = new Dictionary<string, string>(StringComparer.Ordinal);
            if (File.Exists(_path))
            {
                try
                {
                    foreach (var f in PylonValue.Parse(File.ReadAllText(_path)).Fields)
                    {
                        if (f.Value.Kind == PylonValueKind.String) map[f.Key] = f.Value.AsString();
                    }
                }
                catch (PylonException)
                {
                    // A damaged file counts as empty; the next write replaces it.
                }
            }
            return _map = map;
        }

        void Save(Dictionary<string, string> map)
        {
            var fields = new List<KeyValuePair<string, PylonValue>>();
            foreach (var kv in map) fields.Add(new KeyValuePair<string, PylonValue>(kv.Key, kv.Value));
            var dir = Path.GetDirectoryName(Path.GetFullPath(_path));
            if (!string.IsNullOrEmpty(dir)) Directory.CreateDirectory(dir);
            var tmp = _path + ".tmp";
            File.WriteAllText(tmp, PylonValue.Object(fields).ToJson());
            if (File.Exists(_path)) File.Replace(tmp, _path, null);
            else File.Move(tmp, _path);
        }

        public string? Get(string key)
        {
            lock (_lock) return Load().TryGetValue(key, out var v) ? v : null;
        }

        public void Set(string key, string value)
        {
            lock (_lock)
            {
                var map = Load();
                map[key] = value;
                Save(map);
            }
        }

        public void Remove(string key)
        {
            lock (_lock)
            {
                var map = Load();
                if (map.Remove(key)) Save(map);
            }
        }
    }

    /// <summary>Storage key names shared with the TypeScript and Swift clients.</summary>
    public static class StorageKeys
    {
        /// <summary>The session token key for an app name.</summary>
        public static string Token(string appName = "default") =>
            appName == "default" ? "pylon_token" : $"pylon:{appName}:token";
    }
}
