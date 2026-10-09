"""Explorer-style column sorting without rebuilding selection on header events."""
import re
from PySide6.QtCore import Qt
from ..settings import read_settings,save_settings


def natural(value):
    return tuple((1,int(p)) if p.isdigit() else (0,p.casefold()) for p in re.split(r'(\d+)',str(value)))

class HeaderSort:
    def __init__(self,w):
        self.w=w;self.orders={'projects':[3,True],'conversations':[2,True],'unassigned':[2,True]}
        saved=read_settings(w.state_dir/'settings.json').get('column_sort',{})
        if isinstance(saved,dict):
            for mode,value in saved.items():
                if mode in self.orders and isinstance(value,list) and len(value)==2 and type(value[0])==int and 0<=value[0]<4 and type(value[1])==bool:self.orders[mode]=value
        header=w.project_list.header();header.setSectionsClickable(True);header.sectionClicked.connect(self.clicked)
        header.setToolTip('열 제목 클릭: 오름차순 / 내림차순 · 경계선 드래그: 너비 조절')
    def clicked(self,column):
        if self.w.mode not in self.orders:return
        old,descending=self.orders[self.w.mode];self.orders[self.w.mode]=[column,not descending if old==column else False]
        self.w.filter_list()
        try:save_settings(self.w.state_dir/'settings.json',{'column_sort':self.orders})
        except OSError:self.w.footer.setText('정렬은 적용됐지만 설정을 저장하지 못했습니다.')
    def indicator(self):
        header=self.w.project_list.header();header.setSortIndicatorShown(self.w.mode in self.orders)
        if self.w.mode in self.orders:
            column,descending=self.orders[self.w.mode];header.setSortIndicator(column,Qt.DescendingOrder if descending else Qt.AscendingOrder)
    def projects(self,projects):
        w=self.w;column,descending=self.orders['projects'];unknown=[];known=[]
        visible={t.id for t in w.visible_conversations()}
        for p in projects:
            result=w.size_scanner.results.get(p.id)
            if column==3 and result is None:unknown.append(p);continue
            value=[natural(p.name),natural(str(p.roots[0]) if p.roots else ''),sum(t.id in visible for t in w.project_threads(p.id)),result.total if result else 0][column]
            known.append((value,natural(p.name),p.id,p))
        return [entry[-1] for entry in sorted(sorted(known,key=lambda e:e[1:3]),key=lambda e:e[0],reverse=descending)]+sorted(unknown,key=lambda p:natural(p.name))
    def chats(self,threads,names,project_only=False):
        if project_only:return sorted(threads,key=lambda t:(-t.updated_at,t.id))
        column,descending=self.orders.get(self.w.mode,self.orders['conversations'])
        def key(t):
            name=names.get(self.w.membership.projects.get(t.id),'')
            value=[natural(t.title),natural(name),t.updated_at,(t.archived,t.internal)][column]
            return value,natural(t.title),t.id
        return sorted(sorted(threads,key=lambda t:(natural(t.title),t.id)),key=lambda t:key(t)[0],reverse=descending)
