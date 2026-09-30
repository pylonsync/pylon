#nullable enable
using System.Collections.Generic;

namespace Pylon
{
    /// <summary>
    /// A session from any sign-in route (<c>/api/auth/guest</c>,
    /// <c>/api/auth/magic/verify</c>, <c>/api/auth/password/*</c>,
    /// <c>/api/auth/refresh</c>).
    /// </summary>
    public sealed class SessionResponse
    {
        public string Token { get; }
        public string? UserId { get; }
        /// <summary>Unix seconds when the session expires, when the route reports it.</summary>
        public long? ExpiresAt { get; }
        /// <summary>True for a guest session.</summary>
        public bool Guest { get; }

        public SessionResponse(string token, string? userId, long? expiresAt, bool guest)
        {
            Token = token;
            UserId = userId;
            ExpiresAt = expiresAt;
            Guest = guest;
        }

        public static SessionResponse FromValue(PylonValue v)
        {
            var token = v["token"].AsStringOr(null);
            if (string.IsNullOrEmpty(token)) throw PylonException.Decoding("session response has no token");
            var exp = v["expires_at"];
            return new SessionResponse(
                token!,
                v["user_id"].AsStringOr(null),
                exp.IsNumber ? (long?)(long)exp.AsDouble() : null,
                v["guest"].Kind == PylonValueKind.Bool && v["guest"].AsBool());
        }
    }

    /// <summary><c>GET /api/auth/me</c>. <see cref="UserId"/> is null for an anonymous caller.</summary>
    public sealed class ResolvedSession
    {
        public string? UserId { get; }
        public string? TenantId { get; }
        public bool IsAdmin { get; }
        public IReadOnlyList<string> Roles { get; }
        public string? AvatarUrl { get; }

        public ResolvedSession(string? userId, string? tenantId, bool isAdmin, IReadOnlyList<string> roles, string? avatarUrl)
        {
            UserId = userId;
            TenantId = tenantId;
            IsAdmin = isAdmin;
            Roles = roles;
            AvatarUrl = avatarUrl;
        }

        public static ResolvedSession FromValue(PylonValue v)
        {
            var roles = new List<string>();
            foreach (var r in v["roles"].Items)
            {
                if (r.Kind == PylonValueKind.String) roles.Add(r.AsString());
            }
            var admin = v["is_admin"];
            return new ResolvedSession(
                v["user_id"].AsStringOr(null),
                v["tenant_id"].AsStringOr(null),
                admin.Kind == PylonValueKind.Bool && admin.AsBool(),
                roles,
                v["avatar_url"].AsStringOr(null));
        }
    }

    /// <summary>One page of <c>GET /api/entities/&lt;entity&gt;/cursor</c>.</summary>
    public sealed class CursorPage
    {
        public IReadOnlyList<PylonValue> Data { get; }
        /// <summary>Pass as <c>after</c> for the next page; null on the last page.</summary>
        public string? NextCursor { get; }
        public bool HasMore { get; }

        public CursorPage(IReadOnlyList<PylonValue> data, string? nextCursor, bool hasMore)
        {
            Data = data;
            NextCursor = nextCursor;
            HasMore = hasMore;
        }

        public static CursorPage FromValue(PylonValue v)
        {
            var more = v["has_more"];
            return new CursorPage(
                v["data"].Items,
                v["next_cursor"].AsStringOr(null),
                more.Kind == PylonValueKind.Bool && more.AsBool());
        }
    }
}
