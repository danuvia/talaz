#pragma once

// Mods load before the launcher fills in libGLESv2, so linking against it
// fails. ImGui's backend is compiled with this header forced in, which turns
// every GL call into a pointer that gl::load() fills from eglGetProcAddress
// once the game has a context.
#include <GLES3/gl3.h>

#define TALAZ_GL_FUNCS(X) \
    X(PFNGLGETSTRINGPROC, glGetString) \
    X(PFNGLGETINTEGERVPROC, glGetIntegerv) \
    X(PFNGLBINDBUFFERPROC, glBindBuffer) \
    X(PFNGLCREATESHADERPROC, glCreateShader) \
    X(PFNGLSHADERSOURCEPROC, glShaderSource) \
    X(PFNGLCOMPILESHADERPROC, glCompileShader) \
    X(PFNGLCREATEPROGRAMPROC, glCreateProgram) \
    X(PFNGLATTACHSHADERPROC, glAttachShader) \
    X(PFNGLLINKPROGRAMPROC, glLinkProgram) \
    X(PFNGLGETPROGRAMIVPROC, glGetProgramiv) \
    X(PFNGLDETACHSHADERPROC, glDetachShader) \
    X(PFNGLDELETESHADERPROC, glDeleteShader) \
    X(PFNGLGETUNIFORMLOCATIONPROC, glGetUniformLocation) \
    X(PFNGLGETATTRIBLOCATIONPROC, glGetAttribLocation) \
    X(PFNGLGENBUFFERSPROC, glGenBuffers) \
    X(PFNGLBINDTEXTUREPROC, glBindTexture) \
    X(PFNGLGETPROGRAMINFOLOGPROC, glGetProgramInfoLog) \
    X(PFNGLGENTEXTURESPROC, glGenTextures) \
    X(PFNGLTEXPARAMETERIPROC, glTexParameteri) \
    X(PFNGLPIXELSTOREIPROC, glPixelStorei) \
    X(PFNGLTEXIMAGE2DPROC, glTexImage2D) \
    X(PFNGLACTIVETEXTUREPROC, glActiveTexture) \
    X(PFNGLISENABLEDPROC, glIsEnabled) \
    X(PFNGLBUFFERDATAPROC, glBufferData) \
    X(PFNGLBUFFERSUBDATAPROC, glBufferSubData) \
    X(PFNGLSCISSORPROC, glScissor) \
    X(PFNGLDRAWELEMENTSPROC, glDrawElements) \
    X(PFNGLISPROGRAMPROC, glIsProgram) \
    X(PFNGLUSEPROGRAMPROC, glUseProgram) \
    X(PFNGLBLENDEQUATIONSEPARATEPROC, glBlendEquationSeparate) \
    X(PFNGLBLENDFUNCSEPARATEPROC, glBlendFuncSeparate) \
    X(PFNGLENABLEPROC, glEnable) \
    X(PFNGLDISABLEPROC, glDisable) \
    X(PFNGLVIEWPORTPROC, glViewport) \
    X(PFNGLBLENDEQUATIONPROC, glBlendEquation) \
    X(PFNGLUNIFORM1IPROC, glUniform1i) \
    X(PFNGLUNIFORMMATRIX4FVPROC, glUniformMatrix4fv) \
    X(PFNGLENABLEVERTEXATTRIBARRAYPROC, glEnableVertexAttribArray) \
    X(PFNGLVERTEXATTRIBPOINTERPROC, glVertexAttribPointer) \
    X(PFNGLGETSHADERIVPROC, glGetShaderiv) \
    X(PFNGLGETSHADERINFOLOGPROC, glGetShaderInfoLog) \
    X(PFNGLBINDVERTEXARRAYPROC, glBindVertexArray) \
    X(PFNGLGENVERTEXARRAYSPROC, glGenVertexArrays) \
    X(PFNGLDELETEVERTEXARRAYSPROC, glDeleteVertexArrays) \
    X(PFNGLBINDSAMPLERPROC, glBindSampler) \
    X(PFNGLDELETEBUFFERSPROC, glDeleteBuffers) \
    X(PFNGLDELETEPROGRAMPROC, glDeleteProgram) \
    X(PFNGLDELETETEXTURESPROC, glDeleteTextures) \
    X(PFNGLGETERRORPROC, glGetError) \
    X(PFNGLDISABLEVERTEXATTRIBARRAYPROC, glDisableVertexAttribArray) \
    X(PFNGLCOPYTEXIMAGE2DPROC, glCopyTexImage2D) \
    X(PFNGLBINDATTRIBLOCATIONPROC, glBindAttribLocation) \
    X(PFNGLUNIFORM1FPROC, glUniform1f) \

#define TALAZ_GL_DECLARE(type, name) extern type tc_##name;
TALAZ_GL_FUNCS(TALAZ_GL_DECLARE)
#undef TALAZ_GL_DECLARE

namespace gl {
// Returns false if the context is missing any of the functions above
bool load();
}  // namespace gl

#define glGetString tc_glGetString
#define glGetIntegerv tc_glGetIntegerv
#define glBindBuffer tc_glBindBuffer
#define glCreateShader tc_glCreateShader
#define glShaderSource tc_glShaderSource
#define glCompileShader tc_glCompileShader
#define glCreateProgram tc_glCreateProgram
#define glAttachShader tc_glAttachShader
#define glLinkProgram tc_glLinkProgram
#define glGetProgramiv tc_glGetProgramiv
#define glDetachShader tc_glDetachShader
#define glDeleteShader tc_glDeleteShader
#define glGetUniformLocation tc_glGetUniformLocation
#define glGetAttribLocation tc_glGetAttribLocation
#define glGenBuffers tc_glGenBuffers
#define glBindTexture tc_glBindTexture
#define glGetProgramInfoLog tc_glGetProgramInfoLog
#define glGenTextures tc_glGenTextures
#define glTexParameteri tc_glTexParameteri
#define glPixelStorei tc_glPixelStorei
#define glTexImage2D tc_glTexImage2D
#define glActiveTexture tc_glActiveTexture
#define glIsEnabled tc_glIsEnabled
#define glBufferData tc_glBufferData
#define glBufferSubData tc_glBufferSubData
#define glScissor tc_glScissor
#define glDrawElements tc_glDrawElements
#define glIsProgram tc_glIsProgram
#define glUseProgram tc_glUseProgram
#define glBlendEquationSeparate tc_glBlendEquationSeparate
#define glBlendFuncSeparate tc_glBlendFuncSeparate
#define glEnable tc_glEnable
#define glDisable tc_glDisable
#define glViewport tc_glViewport
#define glBlendEquation tc_glBlendEquation
#define glUniform1i tc_glUniform1i
#define glUniformMatrix4fv tc_glUniformMatrix4fv
#define glEnableVertexAttribArray tc_glEnableVertexAttribArray
#define glVertexAttribPointer tc_glVertexAttribPointer
#define glGetShaderiv tc_glGetShaderiv
#define glGetShaderInfoLog tc_glGetShaderInfoLog
#define glBindVertexArray tc_glBindVertexArray
#define glGenVertexArrays tc_glGenVertexArrays
#define glDeleteVertexArrays tc_glDeleteVertexArrays
#define glBindSampler tc_glBindSampler
#define glDeleteBuffers tc_glDeleteBuffers
#define glDeleteProgram tc_glDeleteProgram
#define glDeleteTextures tc_glDeleteTextures
#define glGetError tc_glGetError
#define glDisableVertexAttribArray tc_glDisableVertexAttribArray
#define glCopyTexImage2D tc_glCopyTexImage2D
#define glBindAttribLocation tc_glBindAttribLocation
#define glUniform1f tc_glUniform1f
