package pro.trailscope.servlet;

import jakarta.servlet.ServletOutputStream;
import jakarta.servlet.WriteListener;
import jakarta.servlet.http.HttpServletResponse;
import jakarta.servlet.http.HttpServletResponseWrapper;
import java.io.IOException;
import java.io.PrintWriter;
import java.io.Writer;

/**
 * Copies the first {@code maxBodyBytes + 1} bytes of the response body as it is written, whatever
 * the response size, counts the bytes, and notices when the client went away.
 */
final class CapturingResponse extends HttpServletResponseWrapper {
  private final BoundedBuffer body;
  private ServletOutputStream stream;
  private PrintWriter writer;
  private volatile boolean clientGone;

  CapturingResponse(HttpServletResponse response, int cap) {
    super(response);
    this.body = new BoundedBuffer(cap);
  }

  /** The kept bytes or text, or null. */
  Object capturedBody() {
    return body.content();
  }

  long bytesWritten() {
    return body.total();
  }

  boolean clientGone() {
    return clientGone;
  }

  void noteFailure(Throwable t) {
    if (isClientAbort(t)) {
      clientGone = true;
    }
  }

  @Override
  public ServletOutputStream getOutputStream() throws IOException {
    if (stream == null) {
      stream = new TeeOutputStream(super.getOutputStream());
    }
    return stream;
  }

  @Override
  public PrintWriter getWriter() throws IOException {
    if (writer == null) {
      writer = new PrintWriter(new TeeWriter(super.getWriter()), false);
    }
    return writer;
  }

  @Override
  public void reset() {
    super.reset();
    body.reset();
  }

  @Override
  public void resetBuffer() {
    super.resetBuffer();
    body.reset();
  }

  /** Tomcat, Jetty and Undertow report a vanished client with these. */
  static boolean isClientAbort(Throwable t) {
    for (Throwable cur = t; cur != null; cur = cur.getCause() == cur ? null : cur.getCause()) {
      String name = cur.getClass().getSimpleName();
      if (name.equals("ClientAbortException")
          || name.equals("EofException")
          || name.equals("AsyncRequestNotUsableException")) {
        return true;
      }
      if (cur instanceof IOException) {
        String m = cur.getMessage();
        if (m != null
            && (m.contains("Broken pipe")
                || m.contains("Connection reset by peer")
                || m.contains("connection was aborted"))) {
          return true;
        }
      }
    }
    return false;
  }

  private final class TeeOutputStream extends ServletOutputStream {
    private final ServletOutputStream out;

    TeeOutputStream(ServletOutputStream out) {
      this.out = out;
    }

    @Override
    public void write(int b) throws IOException {
      try {
        out.write(b);
      } catch (IOException e) {
        noteFailure(e);
        throw e;
      }
      body.write(b);
    }

    @Override
    public void write(byte[] b, int off, int len) throws IOException {
      try {
        out.write(b, off, len);
      } catch (IOException e) {
        noteFailure(e);
        throw e;
      }
      body.write(b, off, len);
    }

    @Override
    public void flush() throws IOException {
      try {
        out.flush();
      } catch (IOException e) {
        noteFailure(e);
        throw e;
      }
    }

    @Override
    public void close() throws IOException {
      out.close();
    }

    @Override
    public boolean isReady() {
      return out.isReady();
    }

    @Override
    public void setWriteListener(WriteListener writeListener) {
      out.setWriteListener(writeListener);
    }
  }

  private final class TeeWriter extends Writer {
    private final PrintWriter out;

    TeeWriter(PrintWriter out) {
      this.out = out;
    }

    @Override
    public void write(int c) {
      out.write(c);
      body.write((char) c);
    }

    @Override
    public void write(char[] cbuf, int off, int len) {
      out.write(cbuf, off, len);
      body.write(cbuf, off, len);
    }

    @Override
    public void write(String str, int off, int len) {
      out.write(str, off, len);
      body.write(str, off, len);
    }

    @Override
    public void flush() {
      out.flush();
    }

    @Override
    public void close() {
      out.close();
    }
  }
}
