#nullable enable
using System;
using System.Collections.Generic;

namespace Pylon
{
    /// <summary>Which rows a <see cref="LiveQuery"/> holds, and in what order.</summary>
    public sealed class LiveQueryOptions
    {
        /// <summary>
        /// Field equality, for example <c>PylonValue.Object(("partyId", id))</c>: a row
        /// matches when every listed field equals the given value.
        /// </summary>
        public PylonValue? Where { get; set; }

        /// <summary>Any other test, run after <see cref="Where"/>.</summary>
        public Func<PylonValue, bool>? Filter { get; set; }

        /// <summary>The field to sort by. Rows without it sort first. Default: the row id.</summary>
        public string? OrderBy { get; set; }

        public bool Descending { get; set; }

        /// <summary>Keep at most this many rows, after sorting.</summary>
        public int? Limit { get; set; }
    }

    /// <summary>What one update of a <see cref="LiveQuery"/> changed.</summary>
    public sealed class LiveQueryChange
    {
        /// <summary>Ids of rows that entered the result.</summary>
        public IReadOnlyList<string> Added { get; }
        /// <summary>Ids of rows still in the result whose fields changed.</summary>
        public IReadOnlyList<string> Updated { get; }
        /// <summary>Ids of rows that left the result.</summary>
        public IReadOnlyList<string> Removed { get; }

        internal LiveQueryChange(IReadOnlyList<string> added, IReadOnlyList<string> updated, IReadOnlyList<string> removed)
        {
            Added = added;
            Updated = updated;
            Removed = removed;
        }
    }

    /// <summary>
    /// Rows of one entity that the server keeps current. Create it with
    /// <see cref="PylonClient.Live"/>. <see cref="Rows"/> and
    /// <see cref="Changed"/> change on the client's dispatcher (Unity's main
    /// thread when the client was made there). Dispose it to stop.
    ///
    /// The server applies the entity's read policies and field redaction:
    /// a row the caller may not read never arrives. Writes go through
    /// server functions; the change comes back here.
    /// </summary>
    public sealed class LiveQuery : IDisposable
    {
        readonly LiveSync _sync;
        Dictionary<string, PylonValue> _byId = new Dictionary<string, PylonValue>(StringComparer.Ordinal);

        public string Entity { get; }
        public LiveQueryOptions Options { get; }

        /// <summary>The matching rows, in order. Replaced (not changed in place) on each update.</summary>
        public IReadOnlyList<PylonValue> Rows { get; private set; } = new PylonValue[0];

        /// <summary>True once the rows reflect a completed pull from the server.</summary>
        public bool Synced { get; private set; }

        /// <summary>Runs after <see cref="Rows"/> changes, and once when the first pull completes.</summary>
        public event Action<LiveQueryChange>? Changed;

        internal LiveQuery(LiveSync sync, string entity, LiveQueryOptions options)
        {
            _sync = sync;
            Entity = entity;
            Options = options;
        }

        /// <summary>The row with this id, if it is in the result.</summary>
        public PylonValue? Get(string id) => _byId.TryGetValue(id, out var row) ? row : null;

        /// <summary>Runs on the dispatcher with a result computed on the network thread.</summary>
        internal void Deliver(IReadOnlyList<PylonValue> rows, bool synced)
        {
            var next = new Dictionary<string, PylonValue>(rows.Count, StringComparer.Ordinal);
            foreach (var r in rows) next[r["id"].AsStringOr("") ?? ""] = r;
            var added = new List<string>();
            var updated = new List<string>();
            var removed = new List<string>();
            foreach (var kv in next)
            {
                if (!_byId.TryGetValue(kv.Key, out var old)) added.Add(kv.Key);
                else if (!old.Equals(kv.Value)) updated.Add(kv.Key);
            }
            foreach (var id in _byId.Keys)
            {
                if (!next.ContainsKey(id)) removed.Add(id);
            }
            var orderChanged = !SameOrder(Rows, rows);
            var firstSync = synced && !Synced;
            _byId = next;
            Rows = rows;
            Synced = synced;
            if (added.Count > 0 || updated.Count > 0 || removed.Count > 0 || orderChanged || firstSync)
                Changed?.Invoke(new LiveQueryChange(added, updated, removed));
        }

        static bool SameOrder(IReadOnlyList<PylonValue> a, IReadOnlyList<PylonValue> b)
        {
            if (a.Count != b.Count) return false;
            for (var i = 0; i < a.Count; i++)
            {
                if (!string.Equals(a[i]["id"].AsStringOr(null), b[i]["id"].AsStringOr(null), StringComparison.Ordinal))
                    return false;
            }
            return true;
        }

        /// <summary>The rows of <paramref name="table"/> that match, sorted and limited.</summary>
        internal List<PylonValue> Select(IEnumerable<PylonValue> table)
        {
            var rows = new List<PylonValue>();
            foreach (var row in table)
            {
                if (Matches(row)) rows.Add(row);
            }
            var field = Options.OrderBy ?? "id";
            rows.Sort((x, y) =>
            {
                var c = Compare(x[field], y[field]);
                if (c == 0) c = string.CompareOrdinal(x["id"].AsStringOr(""), y["id"].AsStringOr(""));
                return Options.Descending ? -c : c;
            });
            if (Options.Limit is int limit && rows.Count > limit) rows.RemoveRange(limit, rows.Count - limit);
            return rows;
        }

        bool Matches(PylonValue row)
        {
            if (Options.Where is PylonValue where)
            {
                foreach (var f in where.Fields)
                {
                    if (!row[f.Key].Equals(f.Value)) return false;
                }
            }
            return Options.Filter == null || Options.Filter(row);
        }

        /// <summary>Nulls first, then numbers, then strings, then anything else by its JSON.</summary>
        static int Compare(PylonValue a, PylonValue b)
        {
            int Rank(PylonValue v) => v.IsNull ? 0 : v.IsNumber ? 1 : v.Kind == PylonValueKind.String ? 2 : 3;
            var ra = Rank(a);
            var rb = Rank(b);
            if (ra != rb) return ra.CompareTo(rb);
            switch (ra)
            {
                case 0: return 0;
                case 1: return a.AsDouble().CompareTo(b.AsDouble());
                case 2: return string.CompareOrdinal(a.AsString(), b.AsString());
                default: return string.CompareOrdinal(a.ToJson(), b.ToJson());
            }
        }

        public void Dispose() => _sync.Remove(this);
    }
}
