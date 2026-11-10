/** Error raised by the backend client (and by the demo data source). */
export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }

  /** The backend could not be reached at all (network error, CORS, server down). */
  get unreachable(): boolean {
    return this.status === 0;
  }
}
