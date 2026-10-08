"""Package the approved PNG into a multi-resolution Windows ICO using Qt."""
import argparse
import struct
from pathlib import Path
from PySide6.QtCore import QBuffer,QIODevice,Qt
from PySide6.QtGui import QImage


def build(source,destination):
    image=QImage(str(source))
    if image.isNull() or image.width()!=image.height():
        raise ValueError('A valid square PNG is required.')
    sizes=(16,24,32,48,64,128,256);images=[]
    for size in sizes:
        frame=image.scaled(size,size,Qt.IgnoreAspectRatio,Qt.SmoothTransformation)
        buffer=QBuffer();buffer.open(QIODevice.WriteOnly)
        if not frame.save(buffer,'PNG'):raise ValueError('Could not encode icon frame.')
        images.append(bytes(buffer.data()))
    offset=6+16*len(images);entries=[]
    for size,payload in zip(sizes,images):
        entries.append(struct.pack('<BBBBHHII',size%256,size%256,0,0,1,32,len(payload),offset))
        offset+=len(payload)
    destination.write_bytes(struct.pack('<HHH',0,1,len(images))+b''.join(entries)+b''.join(images))
    print(str(destination))


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('source',type=Path);parser.add_argument('destination',type=Path)
    args=parser.parse_args();build(args.source,args.destination)
