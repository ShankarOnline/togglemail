#!/bin/sh
# Rebuilds ui/demo.html (the website demo) from ui/index.html: the same page with demo.js loaded first.
cd "$(dirname "$0")/../ui" || exit 1
sed -e 's|  <script src="app.js"></script>|  <script src="demo.js"></script>\n  <script src="app.js"></script>|' \
    -e 's|<title>ToggleMail</title>|<title>ToggleMail demo</title>|' index.html > demo.html
