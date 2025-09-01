#!/bin/sh
mount -t proc proc /proc

# Step 1: run dbus
export DBUS_VERBOSE=1
export DBUS_DEBUG_OUTPUT=1
export NO_AT_BRIDGE=1
chmod 755 /run/dbus
eval "$(/usr/bin/dbus-launch --sh-syntax)"

if command -v dconf-service >/dev/null 2>&1; then
  dconf-service > ~/dconf.log 2>&1 & echo $! > /run/dconf-service.pid &
fi

# Step 2: run Xorg
Xorg :0 -modulepath /usr/lib/xorg/modules -config /usr/share/X11/xorg.conf.d/10-fbdev.conf -logverbose 6 -logfile /var/xorg_debug.log -novtswitch -keeptty -keyboard keyboard -pointer mouse0 -xkbdir /usr/share/X11/xkb & echo $! > /run/xorg.pid &

# xfconfd will be called by xfsettingsd, thus no need to run it manually
export DISPLAY=:0
export GDK_BACKEND=x11
export GDK_CORE_DEVICE_EVENTS=1

export GTK_THEME="Adwaita"
export ICON_THEME="hicolor"
export XDG_DATA_DIRS="/usr/share:/usr/local/share"
export XDG_DATA_DIRS="/usr/share:/usr/local/share"

export GDK_PIXBUF_MODULE_FILE=/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache
export GDK_PIXBUF_MODULEDIR=/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders
export GIO_MODULE_DIR=/usr/lib/gio/modules
export GIO_EXTRA_MODULES=/usr/lib/gio/modules

#for debug
export G_MESSAGES_DEBUG=all

# Start tumbler (thumbnails used by settings dialog)
if command -v tumblerd >/dev/null 2>&1; then
  tumblerd -n > ~/tumblerd.log 2>&1 &
fi
xfsettingsd > ~/xfsettingsd.log 2>&1 & echo $! > /run/xfsettingsd.pid &

#Step 3: run xfwm4
export XFWM4_LOG_FILE="/xfwm4.log"
xfwm4 --compositor=off & echo $! > /run/xfwm4.pid &
#strace -o xfwm4_strace.log /usr/bin/xfwm4 --compositor=off -d &
#In asterinas /dev/null seems not working well. So needs to use "-d"

# Wait for EWMH props so xfdesktop doesn’t start “too early”
for i in $(seq 1 50); do
  if xprop -root _NET_NUMBER_OF_DESKTOPS >/dev/null 2>&1; then break; fi
  sleep 0.1
done

#Step 4: run xfdesktop
xfdesktop --enable-debug > ~/xfdesktop.log 2>&1 & echo $! > /run/xfdesktop.pid &
#strace -o xfdesktop_strace.log /usr/bin/xfdesktop --enable-debug > ~/xfdesktop.log 2>&1 &

#Step 5: run xfce4-panel
xfce4-panel > ~/xfce4-panel.log 2>&1 & echo $! > /run/xfce4-panel.pid &
