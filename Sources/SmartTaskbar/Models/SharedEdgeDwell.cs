namespace SmartTaskbar
{
    internal struct SharedEdgeDwell
    {
        private const int DelayMs = 180;
        private const int RetryDelayMs = 400;
        private const int MaxRevealRequests = 2;
        private IntPtr _candidate;
        private long _enteredAt;
        private long _retryAt;
        private int _revealRequests;

        public bool IsReady(IntPtr bar, long now)
        {
            if (_candidate != bar)
            {
                _candidate = bar;
                _enteredAt = now;
                _retryAt = now;
                _revealRequests = 0;
            }

            return bar != IntPtr.Zero && now - _enteredAt >= DelayMs;
        }

        public bool TryRequestReveal(long now)
        {
            if (!IsReady(_candidate, now) || now < _retryAt || _revealRequests >= MaxRevealRequests)
                return false;

            _revealRequests++;
            _retryAt = now + RetryDelayMs;
            return true;
        }
    }
}
