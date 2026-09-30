import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "xrs"
  ipcTarget: "xrs"
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null

  property bool running: false
  property string activeNode: "None"
  property string activeNodeId: ""
  property int socksPort: 10808
  property int httpPort: 10809
  property bool iranRouting: true
  property bool tunEnabled: false
  property string rulesSummary: ""
  property var nodes: []
  property bool busy: false
  property string statusMsg: ""

  // CLI Integration Processes
  Process {
    id: statusProc
    command: ["xrs", "status", "--json"]
    stdout: StdioCollector {
      id: statusOut
      waitForEnd: true
    }
    onExited: function(exitCode) {
      root.busy = false
      if (exitCode === 0 && statusOut.text.trim().length > 0) {
        try {
          var data = JSON.parse(statusOut.text)
          root.running = data.running === true
          root.activeNode = data.active_node_name || "None"
          root.activeNodeId = data.active_node_id || ""
          root.socksPort = data.socks_port || 10808
          root.httpPort = data.http_port || 10809
          root.iranRouting = data.iran_routing !== false
          root.tunEnabled = data.tun_enabled === true
          root.rulesSummary = (data.active_rules || []).join(", ")
          root.nodes = data.nodes || []
        } catch (e) {
          console.log("xrs JSON parse error:", e)
        }
      }
    }
  }

  Process {
    id: actionProc
    property var callback: null
    stdout: StdioCollector { waitForEnd: true }
    onExited: function(exitCode) {
      root.busy = false
      if (actionProc.callback) {
        actionProc.callback()
        actionProc.callback = null
      }
      root.refresh()
    }
  }

  function runCmd(args, cb) {
    if (actionProc.running) return
    root.busy = true
    actionProc.callback = cb || null
    actionProc.command = ["xrs"].concat(args)
    actionProc.running = true
  }

  function refresh() {
    if (statusProc.running) return
    statusProc.command = ["xrs", "status", "--json"]
    statusProc.running = true
  }

  function toggleProxy() {
    runCmd(["toggle"])
  }

  function toggleTun() {
    runCmd(["tun", root.tunEnabled ? "off" : "on"])
  }

  function selectNode(nodeId) {
    runCmd(["node", "select", nodeId])
  }

  function pingNodes() {
    root.statusMsg = "Testing node latency..."
    runCmd(["node", "ping"], function() {
      root.statusMsg = "Latency test finished"
    })
  }

  function updateSubs() {
    root.statusMsg = "Updating subscriptions..."
    runCmd(["sub", "update"], function() {
      root.statusMsg = "Subscriptions updated"
    })
  }

  function openTui() {
    var proc = Qt.createQmlObject('import Quickshell.Io; Process { command: ["foot", "-e", "xrs"] }', root)
    proc.running = true
  }

  Timer {
    interval: 2000
    running: true
    repeat: true
    onTriggered: root.refresh()
  }

  onOpenedChanged: if (opened) {
    root.statusMsg = ""
    root.refresh()
  }

  KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root
    bar: root.bar
    open: root.opened
    contentWidth: panel.fittedContentWidth(Style.space(380))
    contentHeight: panel.fittedContentHeight(stickyHeader.implicitHeight + scrollCol.implicitHeight + footerCol.implicitHeight + Style.space(44), Style.space(560))

    ColumnLayout {
      id: rootCol
      anchors.fill: parent
      spacing: Style.space(10)

      // STICKY header: toggle + active node data, stays visible while scrolling
      Column {
        id: stickyHeader
        Layout.fillWidth: true
        spacing: Style.space(10)

        // 1. Header Row
        RowLayout {
          width: parent.width
          spacing: Style.space(10)

          Text {
            text: "󰖂"
            font.family: Style.font.family
            font.pixelSize: Style.font.title
            color: Color.accent
          }

          Column {
            Layout.fillWidth: true
            spacing: Style.space(2)

            Text {
              text: root.running ? "● CONNECTED" : "○ DISCONNECTED"
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
              font.bold: true
              color: root.running ? Color.accent : Qt.darker(Color.popups.text, 1.6)
            }
          }

          ToggleSwitch {
            checked: root.running
            accent: Color.accent
            onToggled: root.toggleProxy()
          }
        }

        // 2. Active Status & Iran Routing Banner
        BorderSurface {
          width: parent.width
          radius: Style.cornerRadius
          color: Style.hoverFillFor(Color.accent, Color.accent)
          borderSpec: Border.controlSpec("normal", Color.popups.border, Color.accent)
          implicitHeight: heroCol.implicitHeight + Style.space(14)

          Column {
            id: heroCol
            anchors.fill: parent
            anchors.margins: Style.space(8)
            spacing: Style.space(4)

            RowLayout {
              width: parent.width

              Text {
                text: "Active:"
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                color: Qt.darker(Color.popups.text, 1.3)
              }

              Text {
                text: root.activeNode
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: true
                color: Color.accent
                elide: Text.ElideRight
                Layout.fillWidth: true
              }
            }

            RowLayout {
              width: parent.width

              Text {
                text: "TUN Mode:"
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                color: Qt.darker(Color.popups.text, 1.3)
              }

              Text {
                text: root.tunEnabled ? "ENABLED" : "DISABLED"
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                font.bold: true
                color: root.tunEnabled ? Color.accent : Qt.darker(Color.popups.text, 1.6)
              }

              Item { Layout.fillWidth: true }

              Text {
                text: root.tunEnabled ? "[Switch to System Proxy]" : "[Enable TUN Mode]"
                font.family: Style.font.family
                font.pixelSize: Style.font.caption * 0.9
                color: Color.accent
              }

              MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: root.toggleTun()
              }
            }

            RowLayout {
              width: parent.width

              Text {
                text: "Rules:"
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                color: Qt.darker(Color.popups.text, 1.3)
              }

              Text {
                text: root.rulesSummary.length > 0 ? root.rulesSummary : "None"
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                font.bold: true
                color: Color.accent
                elide: Text.ElideRight
                Layout.fillWidth: true
              }
            }
          }
        }

        PanelSeparator { width: parent.width }
      } // stickyHeader

      Flickable {
        id: flick
        Layout.fillWidth: true
        Layout.fillHeight: true
        contentWidth: width
        contentHeight: scrollCol.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height

        Column {
          id: scrollCol
          width: flick.width
          spacing: Style.space(10)

          // 3. Configs & Proxies Section
        PanelSectionHeader {
          text: "CONFIGS & PROXIES (" + root.nodes.length + ")"
        }

        Column {
          id: nodesCol
          width: parent.width
          spacing: Style.space(3)

          Repeater {
            model: root.nodes

            BorderSurface {
              required property var modelData
              required property int index

              width: nodesCol.width
              implicitHeight: Style.space(36)
              radius: Style.cornerRadius
              color: nodeMouse.containsMouse
                ? Style.hoverFillFor(Color.accent, Color.accent)
                : (modelData.is_active ? Style.selectedFillFor(Color.accent, Color.accent) : "transparent")
              borderSpec: modelData.is_active
                ? Border.controlSpec("focus", Color.accent, Color.accent)
                : Border.none()

              RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Style.space(8)
                anchors.rightMargin: Style.space(8)
                spacing: Style.space(8)

                // Radio Indicator
                Rectangle {
                  width: Style.space(12)
                  height: Style.space(12)
                  radius: Style.space(6)
                  color: modelData.is_active ? Color.accent : "transparent"
                  border.color: modelData.is_active ? Color.accent : Qt.darker(Color.popups.text, 1.6)
                  border.width: 1.5
                }

                // Protocol badge
                BorderSurface {
                  radius: Style.space(4)
                  color: Style.hoverFillFor(Color.accent, Color.accent)
                  implicitWidth: protoTxt.implicitWidth + Style.space(6)
                  implicitHeight: Style.space(16)

                  Text {
                    id: protoTxt
                    anchors.centerIn: parent
                    text: modelData.protocol || "VLESS"
                    font.family: Style.font.family
                    font.pixelSize: Style.font.caption * 0.9
                    font.bold: true
                    color: Color.accent
                  }
                }

                // Node Name
                Text {
                  text: modelData.name || "Unnamed"
                  font.family: Style.font.family
                  font.pixelSize: Style.font.body
                  font.bold: modelData.is_active
                  color: modelData.is_active ? Color.accent : Color.popups.text
                  elide: Text.ElideRight
                  Layout.fillWidth: true
                }

                // Ping Latency
                Text {
                  text: modelData.ping_ms > 0 ? (modelData.ping_ms + "ms") : "---"
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                  font.bold: true
                  color: modelData.ping_ms > 0 ? Color.accent : Qt.darker(Color.popups.text, 1.8)
                }
              }

              MouseArea {
                id: nodeMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                  root.selectNode(modelData.id)
                }
              }
            }
          }

          Text {
            visible: root.nodes.length === 0
            text: "No proxy nodes configured.\nRun 'xrs node add <link>' or open TUI."
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            color: Qt.darker(Color.popups.text, 1.5)
            horizontalAlignment: Text.AlignHCenter
            width: parent.width
            topPadding: Style.space(8)
            bottomPadding: Style.space(8)
          }
        }

        // Status message toast
        Text {
          visible: root.statusMsg.length > 0
          text: root.statusMsg
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          font.bold: true
          color: Color.accent
          horizontalAlignment: Text.AlignHCenter
          width: parent.width
        }
        } // scrollCol
      } // Flickable

      // STICKY footer: quick actions pinned to panel bottom
      Column {
        id: footerCol
        Layout.fillWidth: true
        spacing: Style.space(8)

        PanelSeparator { width: parent.width }

        // 4. Quick Actions Row
        RowLayout {
          width: parent.width
          spacing: Style.space(8)

          BorderSurface {
            Layout.fillWidth: true
            implicitHeight: Style.space(30)
            radius: Style.cornerRadius
            color: pingMouse.containsMouse ? Style.hoverFillFor(Color.accent, Color.accent) : "transparent"
            borderSpec: Border.controlSpec("normal", Color.popups.border, Color.accent)

            RowLayout {
              anchors.centerIn: parent
              spacing: Style.space(4)
              Text { text: "󰓅"; font.family: Style.font.family; font.pixelSize: Style.font.caption; color: Color.accent }
              Text {
                text: "Ping"
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                font.bold: true
                color: Color.popups.text
              }
            }

            MouseArea {
              id: pingMouse
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: root.pingNodes()
            }
          }

          BorderSurface {
            Layout.fillWidth: true
            implicitHeight: Style.space(30)
            radius: Style.cornerRadius
            color: updateMouse.containsMouse ? Style.hoverFillFor(Color.accent, Color.accent) : "transparent"
            borderSpec: Border.controlSpec("normal", Color.popups.border, Color.accent)

            RowLayout {
              anchors.centerIn: parent
              spacing: Style.space(4)
              Text { text: "󰑓"; font.family: Style.font.family; font.pixelSize: Style.font.caption; color: Color.accent }
              Text {
                text: "Update"
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                font.bold: true
                color: Color.popups.text
              }
            }

            MouseArea {
              id: updateMouse
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: root.updateSubs()
            }
          }

          BorderSurface {
            Layout.fillWidth: true
            implicitHeight: Style.space(30)
            radius: Style.cornerRadius
            color: tuiMouse.containsMouse ? Style.hoverFillFor(Color.accent, Color.accent) : "transparent"
            borderSpec: Border.controlSpec("normal", Color.popups.border, Color.accent)

            RowLayout {
              anchors.centerIn: parent
              spacing: Style.space(4)
              Text { text: ""; font.family: Style.font.family; font.pixelSize: Style.font.caption; color: Color.accent }
              Text {
                text: "TUI"
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                font.bold: true
                color: Color.popups.text
              }
            }

            MouseArea {
              id: tuiMouse
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: {
                root.openTui()
                root.close()
              }
            }
          }
        }
      } // footerCol
    } // rootCol
  } // panel
} // root
