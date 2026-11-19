namespace SmartTaskbar
{
    internal struct SharedEdgeDwell
    {
        private const int DelayMs = 180;
        private IntPtr _candidate;
        private long _enteredAt;

        public bool IsReady(IntPtr bar, long now)
        {
            if (_candidate != bar)
            {
                _candidate = bar;
                _enteredAt = now;
            }

            return bar != IntPtr.Zero && now - _enteredAt >= DelayMs;
        }
    }
}
