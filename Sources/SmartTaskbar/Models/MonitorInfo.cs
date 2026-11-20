using System.Runtime.InteropServices;

namespace SmartTaskbar
{
    [StructLayout(LayoutKind.Sequential)]
    internal struct MonitorInfo
    {
        public uint Size;
        public TagRect Display;
        public TagRect WorkArea;
        public uint Flags;

        public static MonitorInfo Create() => new() { Size = (uint)Marshal.SizeOf<MonitorInfo>() };
    }
}
