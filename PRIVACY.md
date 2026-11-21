# Privacy

SmartTaskbar stores preferences on your PC and has no telemetry. It reads Windows taskbar, display, and window information to control when the taskbar shows. If you enable the media player, it reads the current media session from Windows.

Opening Settings checks GitHub for the latest release. The optional search redirect downloads its helper when enabled.

The **Web Search** tab passes your query to your default browser when you submit it. SmartTaskbar does not store the query or send it over the network.

The **Search the web** action in Start reads the focused search field when you invoke it and sends the submitted query to your default browser’s configured engine through an embedded WebView2 pane. The pane uses a separate browser data folder beside the SmartTaskbar settings and does not share your default browser’s cookies.
