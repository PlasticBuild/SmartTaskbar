namespace SmartTaskbar
{
    internal struct AutoHideTransition
    {
        private bool? _candidate;
        private long _since;

        public bool IsReady(bool current, bool wanted, long now, int hideDelay = 125)
        {
            if (current == wanted)
            {
                _candidate = null;
                return true;
            }

            if (_candidate != wanted)
            {
                _candidate = wanted;
                _since = now;
            }

            // Wait a little longer before pinning again, in case a move or fullscreen switch is in progress.
            return now - _since >= (wanted ? hideDelay : 200);
        }
    }
}
