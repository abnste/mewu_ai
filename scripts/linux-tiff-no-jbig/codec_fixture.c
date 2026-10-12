// SPDX-License-Identifier: MPL-2.0
#include <tiffio.h>
#include <tif_config.h>
#ifdef JBIG_SUPPORT
#error "JBIG must be disabled"
#endif
#if !defined(JPEG_SUPPORT) || !defined(ZIP_SUPPORT) || !defined(LZMA_SUPPORT) || !defined(ZSTD_SUPPORT) || !defined(WEBP_SUPPORT)
#error "Required original TIFF codec support missing"
#endif
#include <gdk-pixbuf/gdk-pixbuf.h>
#include <gtk/gtk.h>
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>

static void rgb_roundtrip(uint16_t codec) {
    const uint32_t width=32, height=24;
    char name[100]; snprintf(name,sizeof(name),"/packet/evidence/codec-%u.tif",codec);
    TIFF *writer=TIFFOpen(name,"w"); assert(writer);
    assert(TIFFSetField(writer,TIFFTAG_IMAGEWIDTH,width));
    assert(TIFFSetField(writer,TIFFTAG_IMAGELENGTH,height));
    assert(TIFFSetField(writer,TIFFTAG_SAMPLESPERPIXEL,3));
    assert(TIFFSetField(writer,TIFFTAG_BITSPERSAMPLE,8));
    assert(TIFFSetField(writer,TIFFTAG_ORIENTATION,ORIENTATION_TOPLEFT));
    assert(TIFFSetField(writer,TIFFTAG_PLANARCONFIG,PLANARCONFIG_CONTIG));
    assert(TIFFSetField(writer,TIFFTAG_PHOTOMETRIC,PHOTOMETRIC_RGB));
    assert(TIFFSetField(writer,TIFFTAG_COMPRESSION,codec));
    assert(TIFFSetField(writer,TIFFTAG_ROWSPERSTRIP,height));
    if(codec==COMPRESSION_JPEG) { assert(TIFFSetField(writer,TIFFTAG_JPEGQUALITY,95)); assert(TIFFSetField(writer,TIFFTAG_JPEGCOLORMODE,JPEGCOLORMODE_RGB)); }
    if(codec==COMPRESSION_WEBP) assert(TIFFSetField(writer,TIFFTAG_WEBP_LOSSLESS,1));
    unsigned char row[32*3];
    for(uint32_t y=0;y<height;y++) { for(uint32_t x=0;x<width;x++) {row[3*x]=20+3*x;row[3*x+1]=30+3*y;row[3*x+2]=80;} assert(TIFFWriteScanline(writer,row,y,0)>=0); }
    TIFFClose(writer);
    TIFF *reader=TIFFOpen(name,"r"); assert(reader);
    uint16_t actual=0; assert(TIFFGetField(reader,TIFFTAG_COMPRESSION,&actual)&&actual==codec);
    int max_error=0;
    for(uint32_t y=0;y<height;y++) {assert(TIFFReadScanline(reader,row,y,0)>=0);for(uint32_t x=0;x<width;x++){int expected[3]={20+3*(int)x,30+3*(int)y,80};for(int c=0;c<3;c++){int e=abs(row[3*x+c]-expected[c]);if(e>max_error)max_error=e;}}}
    TIFFClose(reader);
    assert(max_error<=(codec==COMPRESSION_JPEG?12:0));
    GError *error=NULL; GdkPixbuf *loaded=gdk_pixbuf_new_from_file(name,&error);
    if(!loaded){fprintf(stderr,"gdk TIFF codec%u: %s\n",codec,error?error->message:"unknown");abort();}
    assert(gdk_pixbuf_get_width(loaded)==32&&gdk_pixbuf_get_height(loaded)==24);
    for(uint32_t y=0;y<height;y++){unsigned char *pixels=gdk_pixbuf_get_pixels(loaded)+y*gdk_pixbuf_get_rowstride(loaded);for(uint32_t x=0;x<width;x++){int expected[3]={20+3*(int)x,30+3*(int)y,80};for(int c=0;c<3;c++)assert(abs(pixels[x*gdk_pixbuf_get_n_channels(loaded)+c]-expected[c])<=(codec==COMPRESSION_JPEG?12:0));}}
    g_object_unref(loaded);
    printf("TIFF codec=%u write/read + gdk loader PASS max_error=%d\n",codec,max_error);
}

static void bilevel_roundtrip(uint16_t codec) {
    char name[100];snprintf(name,sizeof(name),"/packet/evidence/bilevel-%u.tif",codec);
    TIFF *t=TIFFOpen(name,"w");assert(t);
    assert(TIFFSetField(t,TIFFTAG_IMAGEWIDTH,32));assert(TIFFSetField(t,TIFFTAG_IMAGELENGTH,24));
    assert(TIFFSetField(t,TIFFTAG_SAMPLESPERPIXEL,1));assert(TIFFSetField(t,TIFFTAG_BITSPERSAMPLE,1));
    assert(TIFFSetField(t,TIFFTAG_PHOTOMETRIC,PHOTOMETRIC_MINISWHITE));assert(TIFFSetField(t,TIFFTAG_PLANARCONFIG,PLANARCONFIG_CONTIG));
    assert(TIFFSetField(t,TIFFTAG_ROWSPERSTRIP,24));assert(TIFFSetField(t,TIFFTAG_COMPRESSION,codec));
    unsigned char row[4];for(int y=0;y<24;y++){memset(row,y%2?0xaa:0x55,sizeof(row));assert(TIFFWriteScanline(t,row,y,0)>=0);}TIFFClose(t);
    t=TIFFOpen(name,"r");assert(t);for(int y=0;y<24;y++){assert(TIFFReadScanline(t,row,y,0)>=0);for(int x=0;x<4;x++)assert(row[x]==(y%2?0xaa:0x55));}TIFFClose(t);
    GError *error=NULL;GdkPixbuf *loaded=gdk_pixbuf_new_from_file(name,&error);assert(loaded);assert(gdk_pixbuf_get_width(loaded)==32&&gdk_pixbuf_get_height(loaded)==24);
    for(int y=0;y<24;y++){unsigned char *pixels=gdk_pixbuf_get_pixels(loaded)+y*gdk_pixbuf_get_rowstride(loaded);for(int x=0;x<32;x++){int black=((y%2?0xaa:0x55)>>(7-(x%8)))&1;for(int c=0;c<3;c++)assert(pixels[x*gdk_pixbuf_get_n_channels(loaded)+c]==(black?0:255));}}g_object_unref(loaded);
    printf("TIFF bilevel=%u write/read + gdk pixels PASS\n",codec);
}

int main(int argc,char **argv) {
    setbuf(stdout,NULL);
    if(argc==2&&!strcmp(argv[1],"--fax-only")) {bilevel_roundtrip(COMPRESSION_CCITTFAX3);bilevel_roundtrip(COMPRESSION_CCITTFAX4);return 0;}
    assert(!TIFFIsCODECConfigured(COMPRESSION_JBIG));
    const uint16_t codecs[]={COMPRESSION_NONE,COMPRESSION_LZW,COMPRESSION_PACKBITS,COMPRESSION_DEFLATE,COMPRESSION_ADOBE_DEFLATE,COMPRESSION_JPEG,COMPRESSION_LZMA,COMPRESSION_ZSTD,COMPRESSION_WEBP};
    for(size_t i=0;i<sizeof(codecs)/sizeof(codecs[0]);i++){assert(TIFFIsCODECConfigured(codecs[i]));rgb_roundtrip(codecs[i]);}
    bilevel_roundtrip(COMPRESSION_CCITTFAX3);bilevel_roundtrip(COMPRESSION_CCITTFAX4);
    GError *error=NULL;GdkPixbuf *original=gdk_pixbuf_new(GDK_COLORSPACE_RGB,FALSE,8,32,24);assert(original);gdk_pixbuf_fill(original,0x376f9dff);
    assert(gdk_pixbuf_save(original,"/packet/evidence/png.png","png",&error,NULL));
    GdkPixbuf *png=gdk_pixbuf_new_from_file("/packet/evidence/png.png",&error);assert(png);assert(gdk_pixbuf_get_width(png)==32);assert(!memcmp(gdk_pixbuf_get_pixels(png),gdk_pixbuf_get_pixels(original),32*3));
    assert(gdk_pixbuf_save(original,"/packet/evidence/jpeg.jpg","jpeg",&error,"quality","95",NULL));
    GdkPixbuf *jpeg=gdk_pixbuf_new_from_file("/packet/evidence/jpeg.jpg",&error);assert(jpeg);assert(gdk_pixbuf_get_width(jpeg)==32&&gdk_pixbuf_get_height(jpeg)==24);
    assert(gtk_init_check(NULL,NULL));GtkWidget *window=gtk_window_new(GTK_WINDOW_TOPLEVEL);gtk_window_set_default_size(GTK_WINDOW(window),32,24);gtk_window_set_title(GTK_WINDOW(window),"TIFF dependency PNG fixture");
    GtkWidget *image=gtk_image_new_from_pixbuf(png);gtk_container_add(GTK_CONTAINER(window),image);gtk_widget_show_all(window);
    while(gtk_events_pending()) {gtk_main_iteration();}
    GdkWindow *native=gtk_widget_get_window(window);assert(native);
    GdkPixbuf *capture=gdk_pixbuf_get_from_window(native,0,0,32,24);assert(capture);unsigned char *center=gdk_pixbuf_get_pixels(capture)+12*gdk_pixbuf_get_rowstride(capture)+16*gdk_pixbuf_get_n_channels(capture);assert(center[0]==0x37&&center[1]==0x6f&&center[2]==0x9d);assert(gdk_pixbuf_save(capture,"/packet/evidence/gtk-capture.png","png",&error,NULL));
    gtk_widget_destroy(window);while(gtk_events_pending())gtk_main_iteration();
    g_object_unref(capture);g_object_unref(jpeg);g_object_unref(png);g_object_unref(original);
    puts("PNG/JPEG read-write + real Xvfb GTK PNG capture PASS; this is not Mewu AppImage GUI acceptance");
    FILE *maps=fopen("/proc/self/maps","r");assert(maps);FILE *out=fopen("/packet/evidence/fixture-maps.txt","w");assert(out);char line[PATH_MAX+200];while(fgets(line,sizeof(line),maps)){fputs(line,out);assert(!strstr(line,"libjbig")&&!strstr(line,"libgpm")&&!strstr(line,"libslang"));}fclose(maps);fclose(out);
    return 0;
}
