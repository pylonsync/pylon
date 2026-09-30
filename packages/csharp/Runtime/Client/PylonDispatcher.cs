#nullable enable
using System;
using System.Threading;

namespace Pylon
{
    /// <summary>
    /// Where the SDK runs your callbacks (shard events, session refresh).
    ///
    /// <see cref="Capture"/> takes the <see cref="SynchronizationContext"/> of
    /// the thread that creates the client. In Unity, create clients on the
    /// main thread and every callback runs there, on the next frame. Without
    /// a context (a console app, a test), callbacks run on the network
    /// thread that received the event.
    /// </summary>
    public sealed class PylonDispatcher
    {
        readonly Action<Action>? _post;

        /// <summary>Run callbacks on the thread that raised them.</summary>
        public static readonly PylonDispatcher Inline = new PylonDispatcher(null);

        PylonDispatcher(Action<Action>? post)
        {
            _post = post;
        }

        /// <summary>Post callbacks with <paramref name="post"/>, for a scheduler of your own.</summary>
        public static PylonDispatcher From(Action<Action> post) =>
            new PylonDispatcher(post ?? throw new ArgumentNullException(nameof(post)));

        /// <summary>Post to the current thread's <see cref="SynchronizationContext"/>, or run inline without one.</summary>
        public static PylonDispatcher Capture()
        {
            var context = SynchronizationContext.Current;
            return context == null ? Inline : new PylonDispatcher(a => context.Post(s => ((Action)s!)(), a));
        }

        public void Post(Action action)
        {
            if (_post == null) action();
            else _post(action);
        }
    }
}
