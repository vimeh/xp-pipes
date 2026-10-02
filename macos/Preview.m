// Bounded native lifecycle check: no saver selection, idle policy, or live
// lock.
#if defined(__APPLE__)
#define GL_SILENCE_DEPRECATION
#import <OpenGL/gl3.h>
#import <ScreenSaver/ScreenSaver.h>
#import <stdlib.h>

@interface ScreenSaverView (XPPipesDiagnostics)
- (NSUInteger)xpFrameCount;
- (BOOL)xpFailed;
- (NSSize)xpPixelSize;
@end

int main(int argc, const char **argv) {
  @autoreleasepool {
    if (argc != 2) {
      fprintf(stderr, "usage: xp-pipes-preview 'XP Pipes.saver'\n");
      return 2;
    }
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    NSBundle *bundle =
        [NSBundle bundleWithPath:[NSString stringWithUTF8String:argv[1]]];
    NSError *error = nil;
    if (![bundle loadAndReturnError:&error]) {
      NSLog(@"bundle load failed: %@", error);
      return 1;
    }
    Class principal = bundle.principalClass;
    if (![principal isSubclassOfClass:[ScreenSaverView class]])
      return 1;
    NSWindow *window =
        [[NSWindow alloc] initWithContentRect:NSMakeRect(100, 100, 640, 480)
                                    styleMask:NSWindowStyleMaskTitled
                                      backing:NSBackingStoreBuffered
                                        defer:NO];
    window.title = @"XP Pipes — bounded preview";
    ScreenSaverView *view =
        [[principal alloc] initWithFrame:NSMakeRect(0, 0, 640, 480)
                               isPreview:YES];
    window.contentView = view;
    [window orderFront:nil];
    [view startAnimation];
    NSWindow *secondWindow =
        [[NSWindow alloc] initWithContentRect:NSMakeRect(800, 100, 320, 240)
                                    styleMask:NSWindowStyleMaskTitled
                                      backing:NSBackingStoreBuffered
                                        defer:NO];
    ScreenSaverView *second =
        [[principal alloc] initWithFrame:NSMakeRect(0, 0, 320, 240)
                               isPreview:NO];
    secondWindow.title = @"XP Pipes — second native view";
    secondWindow.contentView = second;
    [secondWindow orderFront:nil];
    [second startAnimation];
    if (!view.hasConfigureSheet || !view.configureSheet)
      return 1;
    __block NSUInteger stoppedFrames = 0;
    __block BOOL stoppedCleanly = NO;
    [NSTimer
        scheduledTimerWithTimeInterval:2
                               repeats:NO
                                 block:^(NSTimer *timer) {
                                   (void)timer;
                                   [window setContentSize:NSMakeSize(900, 400)];
                                 }];
    [NSTimer scheduledTimerWithTimeInterval:3
                                    repeats:NO
                                      block:^(NSTimer *timer) {
                                        (void)timer;
                                        [view stopAnimation];
                                        stoppedFrames = [view xpFrameCount];
                                      }];
    [NSTimer
        scheduledTimerWithTimeInterval:4
                               repeats:NO
                                 block:^(NSTimer *timer) {
                                   (void)timer;
                                   stoppedCleanly =
                                       !view.isAnimating &&
                                       [view xpFrameCount] == stoppedFrames;
                                   [window setContentSize:NSMakeSize(400, 700)];
                                   [view startAnimation];
                                 }];
    [NSTimer
        scheduledTimerWithTimeInterval:8
                               repeats:NO
                                 block:^(NSTimer *timer) {
                                   (void)timer;
                                   NSSize pixels = [view xpPixelSize];
                                   NSUInteger frames = [view xpFrameCount];
                                   BOOL success = stoppedCleanly &&
                                                  ![view xpFailed] &&
                                                  ![second xpFailed] &&
                                                  [second xpFrameCount] > 0 &&
                                                  frames > stoppedFrames &&
                                                  stoppedFrames > 0 &&
                                                  pixels.width >= 400 &&
                                                  pixels.height >= 700;
                                   for (NSView *child in view.subviews)
                                     if ([child isKindOfClass:[NSOpenGLView
                                                                  class]]) {
                                       NSOpenGLView *glView =
                                           (NSOpenGLView *)child;
                                       [glView.openGLContext
                                               makeCurrentContext];
                                       fprintf(stdout,
                                               "GL=%s renderer=%s error=%u\n",
                                               glGetString(GL_VERSION),
                                               glGetString(GL_RENDERER),
                                               glGetError());
                                     }
                                   [view stopAnimation];
                                   [window orderOut:nil];
                                   [second stopAnimation];
                                   [secondWindow orderOut:nil];
                                   fprintf(
                                       stdout,
                                       "frames=%lu stop/restart=%d "
                                       "Retina-pixels=%.0fx%.0f result=%s\n",
                                       (unsigned long)frames, stoppedCleanly,
                                       pixels.width, pixels.height,
                                       success ? "PASS" : "FAIL");
                                   fflush(stdout);
                                   exit(success ? 0 : 1);
                                 }];
    [NSApp run];
  }
  return 1;
}
#endif
