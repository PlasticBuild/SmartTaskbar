# Privacy

SmartTaskbar stores preferences on your PC and has no telemetry. It reads Windows taskbar, display, and window information to control when the taskbar shows. If you enable the media player, it reads the current media session from Windows.

Opening Settings checks GitHub for the latest release. The optional search redirect downloads its helper when enabled.

The **Web Search** tab passes your query to your default browser when you submit it. SmartTaskbar does not store the query or send it over the network.

SmartTaskbar Start keeps app searches on your PC. The **Web** tab and **Web:** prefix send your web query to your configured search engine after you pause typing. The embedded WebView2 view has a separate browser data folder beside the SmartTaskbar settings. **Open in browser** launches your default browser directly.

While Windows Search has focus, the optional handoff to SmartTaskbar Start uses Windows accessibility to detect the search field and carry over the query when requested.
