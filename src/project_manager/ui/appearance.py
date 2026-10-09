"""Light application palette and a static, softly diffused glass backdrop."""
from pathlib import Path
from PySide6.QtCore import Qt,QPointF
from PySide6.QtGui import QColor,QPalette,QPainter,QLinearGradient,QRadialGradient
from PySide6.QtWidgets import QWidget,QGraphicsDropShadowEffect


def apply_light_theme(app):
    if app.property('codexManagerLightTheme'):return
    app.setStyle('Fusion')
    app.styleHints().setColorScheme(Qt.ColorScheme.Light)
    palette=QPalette()
    for role,value in {
        QPalette.Window:'#F2F5FA',QPalette.WindowText:'#243247',
        QPalette.Base:'#FFFFFF',QPalette.AlternateBase:'#F0F5FC',
        QPalette.Text:'#243247',QPalette.Button:'#F8FAFE',QPalette.ButtonText:'#243247',
        QPalette.Highlight:'#D9E8FC',QPalette.HighlightedText:'#173E79',
        QPalette.ToolTipBase:'#FFFFFF',QPalette.ToolTipText:'#243247',
        QPalette.PlaceholderText:'#66758A',QPalette.Link:'#315FC0',
    }.items():palette.setColor(role,QColor(value))
    for role in (QPalette.Text,QPalette.WindowText,QPalette.ButtonText):
        palette.setColor(QPalette.Disabled,role,QColor('#8B96A7'))
    app.setPalette(palette)
    ui_dir=Path(__file__).parent
    app.setStyleSheet((ui_dir/'theme.qss').read_text(encoding='utf-8').replace('@UI_DIR@',ui_dir.as_posix()))
    app.setProperty('codexManagerLightTheme',True)


class GlassBackdrop(QWidget):
    def paintEvent(self,event):
        painter=QPainter(self)
        base=QLinearGradient(0,0,self.width(),self.height())
        base.setColorAt(0,QColor('#EAF0F9'));base.setColorAt(.5,QColor('#F1F4F9'));base.setColorAt(1,QColor('#E5ECF7'))
        painter.fillRect(self.rect(),base)
        for x,y,r,color in ((.9,.08,.64,'#BCCFF5'),(.18,.96,.6,'#CEC7EB'),(.05,.05,.38,'#D1E5EF')):
            glow=QRadialGradient(QPointF(self.width()*x,self.height()*y),max(self.width(),self.height())*r)
            tint=QColor(color);glow.setColorAt(0,tint);tint.setAlpha(0);glow.setColorAt(1,tint)
            painter.fillRect(self.rect(),glow)


def panel_shadow(widget):
    shadow=QGraphicsDropShadowEffect(widget)
    shadow.setBlurRadius(28);shadow.setOffset(0,7);shadow.setColor(QColor(47,67,105,23))
    widget.setGraphicsEffect(shadow)
