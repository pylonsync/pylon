#nullable enable
using System.Collections.Generic;
using System.Threading;
using UnityEngine;

namespace Pylon.Unity
{
    /// <summary>
    /// Keeps the session token in <see cref="PlayerPrefs"/>, so it survives
    /// restarts. PlayerPrefs works only on the main thread: create this
    /// there. Reads come from a copy in memory; writes reach PlayerPrefs on
    /// the main thread (at once when made there, else on the next frame).
    /// </summary>
    public sealed class PlayerPrefsStorage : IPylonStorage
    {
        readonly object _lock = new object();
        readonly Dictionary<string, string?> _cache = new Dictionary<string, string?>();
        readonly SynchronizationContext? _main;
        readonly int _mainThread;
        readonly string _prefix;

        /// <param name="appName">The client's app name; its token is read now, so reads off the main thread find it.</param>
        /// <param name="prefix">Put before every key in PlayerPrefs.</param>
        public PlayerPrefsStorage(string appName = "default", string prefix = "pylon.")
        {
            _prefix = prefix;
            _main = SynchronizationContext.Current;
            _mainThread = Thread.CurrentThread.ManagedThreadId;
            Get(StorageKeys.Token(appName));
        }

        public string? Get(string key)
        {
            lock (_lock)
            {
                if (_cache.TryGetValue(key, out var cached)) return cached;
            }
            if (Thread.CurrentThread.ManagedThreadId != _mainThread) return null;
            var value = PlayerPrefs.HasKey(_prefix + key) ? PlayerPrefs.GetString(_prefix + key) : null;
            lock (_lock) _cache[key] = value;
            return value;
        }

        public void Set(string key, string value)
        {
            lock (_lock) _cache[key] = value;
            OnMain(() =>
            {
                PlayerPrefs.SetString(_prefix + key, value);
                PlayerPrefs.Save();
            });
        }

        public void Remove(string key)
        {
            lock (_lock) _cache[key] = null;
            OnMain(() =>
            {
                PlayerPrefs.DeleteKey(_prefix + key);
                PlayerPrefs.Save();
            });
        }

        void OnMain(System.Action action)
        {
            if (Thread.CurrentThread.ManagedThreadId == _mainThread || _main == null) action();
            else _main.Post(_ => action(), null);
        }
    }
}
