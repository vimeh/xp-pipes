// Native-only translation unit; the shared engine also builds without Apple
// SDKs.
#if defined(__APPLE__)
#define GL_SILENCE_DEPRECATION
#import "Pipes.h"
#import <OpenGL/gl3.h>
#import <ScreenSaver/ScreenSaver.h>
#include <math.h>

static NSString *const Domain = @"dev.vinay.XPPipes";

@interface VMXPPipesSaverView : ScreenSaverView
@end

@implementation VMXPPipesSaverView {
  NSOpenGLView *_glView;
  void *_engine;
  CGLContextObj _engineContext;
  BOOL _failed;
  NSUInteger _frames;
  NSSize _pixels;
  NSWindow *_sheet;
  NSPopUpButton *_pipes, *_joints, *_quality;
  NSSlider *_speed;
  NSTextField *_textureLabel;
  NSData *_textureBookmark;
}

- (ScreenSaverDefaults *)preferences {
  ScreenSaverDefaults *prefs =
      [ScreenSaverDefaults defaultsForModuleWithName:Domain];
  [prefs registerDefaults:@{
    @"multiple" : @NO,
    @"joints" : @0,
    @"speed" : @16,
    @"quality" : @0
  }];
  return prefs;
}

- (instancetype)initWithFrame:(NSRect)frame isPreview:(BOOL)preview {
  self = [super initWithFrame:frame isPreview:preview];
  if (!self)
    return nil;
  NSOpenGLPixelFormatAttribute attributes[] = {
      NSOpenGLPFAOpenGLProfile, NSOpenGLProfileVersion4_1Core,
      NSOpenGLPFAAccelerated,   NSOpenGLPFAAllowOfflineRenderers,
      NSOpenGLPFAColorSize,     24,
      NSOpenGLPFAAlphaSize,     8,
      NSOpenGLPFADoubleBuffer,  0};
  NSOpenGLPixelFormat *format =
      [[NSOpenGLPixelFormat alloc] initWithAttributes:attributes];
  if (!format) {
    NSLog(@"XP Pipes: OpenGL 4.1 pixel format unavailable");
    _failed = YES;
    return self;
  }
  _glView = [[NSOpenGLView alloc] initWithFrame:self.bounds pixelFormat:format];
  _glView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
  _glView.wantsBestResolutionOpenGLSurface = YES;
  [self addSubview:_glView];
  GLint interval = 1;
  [_glView.openGLContext setValues:&interval
                      forParameter:NSOpenGLContextParameterSwapInterval];
  self.animationTimeInterval = 1.0 / 60.0;
  return self;
}

- (BOOL)isOpaque {
  return YES;
}

- (void)releaseEngine {
  if (_engine) {
    void *engine = _engine;
    _engine = NULL;
    _engineContext = NULL;
    xp_pipes_destroy(engine);
  }
}

- (void)startAnimation {
  [super startAnimation];
  _failed = (_glView == nil);
}

- (void)stopAnimation {
  [super stopAnimation];
  [self releaseEngine];
  // Also retires any partially constructed GL resources after a failed start.
  [_glView clearGLContext];
}

- (void)dealloc {
  [self releaseEngine];
}

- (void)viewDidChangeBackingProperties {
  [super viewDidChangeBackingProperties];
  [_glView.openGLContext update];
}

- (NSURL *)textureURL:(ScreenSaverDefaults *)prefs {
  NSData *data = [prefs dataForKey:@"textureBookmark"];
  if (!data)
    return nil;
  BOOL stale = NO;
  NSError *error = nil;
  NSURL *url = [NSURL
      URLByResolvingBookmarkData:data
                         options:NSURLBookmarkResolutionWithSecurityScope |
                                 NSURLBookmarkResolutionWithoutUI
                   relativeToURL:nil
             bookmarkDataIsStale:&stale
                           error:&error];
  if (!url)
    NSLog(@"XP Pipes: texture bookmark could not be resolved: %@", error);
  return url;
}

- (void)animateOneFrame {
  if (!self.isAnimating || _failed || !_glView.window)
    return;
  NSRect bounds = _glView.bounds;
  NSRect backing = [_glView convertRectToBacking:bounds];
  double w = NSWidth(bounds), h = NSHeight(bounds);
  double pw = NSWidth(backing), ph = NSHeight(backing);
  if (!(isfinite(w) && isfinite(h) && isfinite(pw) && isfinite(ph)) || w < 1 ||
      h < 1 || pw < 1 || ph < 1)
    return;
  if (w > 65535 || h > 65535 || pw > 16384 || ph > 16384) {
    _failed = YES;
    return;
  }
  CGLContextObj context = _glView.openGLContext.CGLContextObj;
  if (_engine && _engineContext != context) {
    NSLog(@"XP Pipes: drawable context replaced %p -> %p", _engineContext,
          context);
    [self releaseEngine];
  }
  [_glView.openGLContext makeCurrentContext];
  [_glView.openGLContext update];
  // AppKit can temporarily detach the drawable during resize/reattachment.
  // Wait for its next callback rather than poisoning a healthy engine.
  glBindFramebuffer(GL_FRAMEBUFFER, 0);
  if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
    return;
  _pixels = NSMakeSize(pw, ph);
  if (!_engine) {
    ScreenSaverDefaults *prefs = [self preferences];
    NSURL *url = [self textureURL:prefs];
    BOOL access = [url startAccessingSecurityScopedResource];
    XPPipesOptions options = {
        .version = 1,
        .multiple = [prefs boolForKey:@"multiple"],
        .joints = (uint32_t)[prefs integerForKey:@"joints"],
        .speed = (uint32_t)[prefs integerForKey:@"speed"],
        .tessellation = (uint32_t)[prefs integerForKey:@"quality"],
        .use_seed = 0,
        .seed = 0,
        .texture_path = url.fileSystemRepresentation};
    if ([prefs dataForKey:@"textureBookmark"] && !url)
      _failed = YES;
    else {
      _engineContext = context;
      _engine = xp_pipes_create(context, &options, (uint32_t)w, (uint32_t)h);
    }
    if (access)
      [url stopAccessingSecurityScopedResource];
    if (!_engine)
      _failed = YES;
  }
  if (!_failed && !xp_pipes_frame(_engine, (uint32_t)w, (uint32_t)h,
                                  (uint32_t)pw, (uint32_t)ph)) {
    NSLog(@"XP Pipes: failed frame %lu at %.0fx%.0f, native context %p engine "
          @"%p framebuffer %x",
          (unsigned long)_frames, pw, ph, context, _engineContext,
          glCheckFramebufferStatus(GL_DRAW_FRAMEBUFFER));
    _failed = YES;
    [self releaseEngine];
  }
  if (_failed) {
    [_glView.openGLContext makeCurrentContext];
    glBindFramebuffer(GL_FRAMEBUFFER, 0);
    glClearColor(0, 0, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT);
  } else
    _frames++;
  [_glView.openGLContext flushBuffer];
  if (_failed)
    [self stopAnimation];
}

// Diagnostics expose only this view's lifecycle, never desktop contents/input.
- (NSUInteger)xpFrameCount {
  return _frames;
}
- (BOOL)xpFailed {
  return _failed;
}
- (NSSize)xpPixelSize {
  return _pixels;
}

- (BOOL)hasConfigureSheet {
  return YES;
}

- (NSTextField *)label:(NSString *)text y:(CGFloat)y {
  NSTextField *label = [NSTextField labelWithString:text];
  label.frame = NSMakeRect(20, y, 140, 24);
  [_sheet.contentView addSubview:label];
  return label;
}

- (NSPopUpButton *)menu:(NSArray<NSString *> *)items
                      y:(CGFloat)y
               selected:(NSInteger)selected {
  NSPopUpButton *menu =
      [[NSPopUpButton alloc] initWithFrame:NSMakeRect(165, y, 235, 28)
                                 pullsDown:NO];
  [menu addItemsWithTitles:items];
  [menu selectItemAtIndex:MAX(0, MIN(selected, (NSInteger)items.count - 1))];
  [_sheet.contentView addSubview:menu];
  return menu;
}

- (NSButton *)button:(NSString *)title action:(SEL)action frame:(NSRect)frame {
  NSButton *button = [NSButton buttonWithTitle:title target:self action:action];
  button.frame = frame;
  [_sheet.contentView addSubview:button];
  return button;
}

- (NSWindow *)configureSheet {
  if (!_sheet) {
    _sheet = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 430, 310)
                                         styleMask:NSWindowStyleMaskTitled
                                           backing:NSBackingStoreBuffered
                                             defer:NO];
    _sheet.title = @"XP Pipes";
    [self label:@"Pipes" y:258];
    _pipes = [self menu:@[ @"Single", @"Multiple" ] y:255 selected:0];
    [self label:@"Joints" y:212];
    _joints = [self menu:@[ @"Elbow", @"Ball", @"Mixed", @"Cycle" ]
                       y:209
                selected:0];
    [self label:@"Tessellation" y:166];
    _quality = [self menu:@[ @"0 — original", @"1", @"2", @"3", @"4" ]
                        y:163
                 selected:0];
    [self label:@"Speed (0–20)" y:120];
    _speed = [NSSlider sliderWithValue:16
                              minValue:0
                              maxValue:20
                                target:nil
                                action:NULL];
    _speed.frame = NSMakeRect(165, 120, 235, 24);
    _speed.numberOfTickMarks = 21;
    _speed.allowsTickMarkValuesOnly = YES;
    [_sheet.contentView addSubview:_speed];
    _textureLabel = [self label:@"Solid" y:80];
    _textureLabel.frame = NSMakeRect(20, 80, 380, 24);
    [self button:@"Choose Texture…"
          action:@selector(chooseTexture:)
           frame:NSMakeRect(20, 44, 155, 28)];
    [self button:@"Use Solid"
          action:@selector(useSolid:)
           frame:NSMakeRect(180, 44, 100, 28)];
    [self button:@"Cancel"
          action:@selector(cancel:)
           frame:NSMakeRect(245, 6, 80, 28)];
    NSButton *ok = [self button:@"OK"
                         action:@selector(save:)
                          frame:NSMakeRect(335, 6, 70, 28)];
    ok.keyEquivalent = @"\r";
  }
  ScreenSaverDefaults *prefs = [self preferences];
  [_pipes selectItemAtIndex:[prefs boolForKey:@"multiple"] ? 1 : 0];
  [_joints selectItemAtIndex:MAX(0, MIN(3, [prefs integerForKey:@"joints"]))];
  [_quality selectItemAtIndex:MAX(0, MIN(4, [prefs integerForKey:@"quality"]))];
  _speed.integerValue = MAX(0, MIN(20, [prefs integerForKey:@"speed"]));
  _textureBookmark = [prefs dataForKey:@"textureBookmark"];
  _textureLabel.stringValue =
      _textureBookmark
          ? ([prefs stringForKey:@"textureName"] ?: @"Custom texture")
          : @"Solid";
  return _sheet;
}

- (void)chooseTexture:(id)sender {
  (void)sender;
  NSOpenPanel *panel = [NSOpenPanel openPanel];
  panel.canChooseDirectories = NO;
  panel.allowsMultipleSelection = NO;
  [panel beginSheetModalForWindow:_sheet
                completionHandler:^(NSModalResponse response) {
                  if (response != NSModalResponseOK)
                    return;
                  NSError *error = nil;
                  NSData *bookmark =
                      [panel.URL bookmarkDataWithOptions:
                                     NSURLBookmarkCreationWithSecurityScope
                          includingResourceValuesForKeys:nil
                                           relativeToURL:nil
                                                   error:&error];
                  if (bookmark) {
                    self->_textureBookmark = bookmark;
                    self->_textureLabel.stringValue =
                        panel.URL.lastPathComponent;
                  } else {
                    NSAlert *alert = [NSAlert alertWithError:error];
                    [alert beginSheetModalForWindow:self->_sheet
                                  completionHandler:nil];
                  }
                }];
}

- (void)useSolid:(id)sender {
  (void)sender;
  _textureBookmark = nil;
  _textureLabel.stringValue = @"Solid";
}
- (void)cancel:(id)sender {
  (void)sender;
  [NSApp endSheet:_sheet];
}
- (void)save:(id)sender {
  (void)sender;
  ScreenSaverDefaults *prefs = [self preferences];
  [prefs setBool:_pipes.indexOfSelectedItem == 1 forKey:@"multiple"];
  [prefs setInteger:_joints.indexOfSelectedItem forKey:@"joints"];
  [prefs setInteger:_quality.indexOfSelectedItem forKey:@"quality"];
  [prefs setInteger:_speed.integerValue forKey:@"speed"];
  if (_textureBookmark) {
    [prefs setObject:_textureBookmark forKey:@"textureBookmark"];
    [prefs setObject:_textureLabel.stringValue forKey:@"textureName"];
  } else {
    [prefs removeObjectForKey:@"textureBookmark"];
    [prefs removeObjectForKey:@"textureName"];
  }
  [prefs synchronize];
  [NSApp endSheet:_sheet];
  [self releaseEngine];
  _failed = NO;
}
@end
#endif
