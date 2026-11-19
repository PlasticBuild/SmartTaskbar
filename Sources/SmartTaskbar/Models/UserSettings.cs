using Microsoft.Win32;
using Windows.Storage;

namespace SmartTaskbar
{
    public static class UserSettings
    {
        private const int BufferRequired = 122;
        private const string StandaloneSettings = @"Software\SmartTaskbar\Desktop";
        private static readonly ApplicationDataContainer? PackageSettings = GetPackageSettings();

        private static ApplicationDataContainer? GetPackageSettings()
        {
            uint length = 0;
            return Fun.GetCurrentPackageFamilyName(ref length, IntPtr.Zero) == BufferRequired
                ? ApplicationData.Current.LocalSettings
                : null;
        }

        public static AutoModeType AutoModeType
        {
            get => (Read(nameof(AutoModeType)) as string) switch
            {
                nameof(AutoModeType.None) => AutoModeType.None,
                nameof(AutoModeType.Dock) => AutoModeType.Dock,
                _ => AutoModeType.Auto
            };
            set => Write(nameof(AutoModeType), value.ToString());
        }

        public static bool ShowTaskbarWhenExit
        {
            get => Read(nameof(ShowTaskbarWhenExit)) switch
            {
                bool value => value,
                int value => value != 0,
                _ => true
            };
            set => Write(nameof(ShowTaskbarWhenExit), value);
        }

        private static object? Read(string name)
        {
            if (PackageSettings != null)
                return PackageSettings.Values.TryGetValue(name, out var value) ? value : null;

            using var key = Registry.CurrentUser.OpenSubKey(StandaloneSettings);
            return key?.GetValue(name);
        }

        private static void Write(string name, object value)
        {
            if (PackageSettings != null)
            {
                PackageSettings.Values[name] = value;
                return;
            }

            using var key = Registry.CurrentUser.CreateSubKey(StandaloneSettings);
            key.SetValue(name, value is bool flag ? (flag ? 1 : 0) : value);
        }
    }
}
