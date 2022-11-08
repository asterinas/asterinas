impl Vmo<Rights> {
    /// Creates a new slice VMO through a set of VMO child options.
    /// 
    /// # Example
    /// 
    /// ```
    /// let parent = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
    /// let child_size = parent.size();
    /// let child = parent.new_slice_child(0..child_size).alloc().unwrap();
    /// assert!(child.size() == child_size);
    /// ``` 
    /// 
    /// For more details on the available options, see `VmoChildOptions`.
    /// 
    /// # Access rights
    /// 
    /// This method requires the Dup right.
    /// 
    /// The new VMO child will be of the same capability flavor as the parent;
    /// so are the access rights.
    pub fn new_slice_child(&self, range: Range<usize>) -> VmoChildOptions<'_, Rights, VmoSliceChild> {
        let dup_self = self.dup()?;
        VmoChildOptions::new_slice(dup_self, range)
    }

    /// Creates a new COW VMO through a set of VMO child options.
    /// 
    /// # Example
    /// 
    /// ```
    /// let parent = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
    /// let child_size = 2 * parent.size();
    /// let child = parent.new_cow_child(0..child_size).alloc().unwrap();
    /// assert!(child.size() == child_size);
    /// ``` 
    /// 
    /// For more details on the available options, see `VmoChildOptions`.
    /// 
    /// # Access rights
    /// 
    /// This method requires the Dup right.
    /// 
    /// The new VMO child will be of the same capability flavor as the parent.
    /// The child will be given the access rights of the parent
    /// plus the Write right.
    pub fn new_cow_child(&self, range: Range<usize>) -> VmoChildOptions<'_, Rights, VmoCowChild> {
        let dup_self = self.dup()?;
        VmoChildOptions::new_cow(dup_self, range)
    }

    /// Commits the pages specified in the range (in bytes).
    /// 
    /// The range must be within the size of the VMO.
    /// 
    /// The start and end addresses will be rounded down and up to page boundaries.
    /// 
    /// # Access rights
    ///
    /// The method requires the Write right. 
    pub fn commit(&self, range: Range<usize>) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.commit(range)
    }

    /// Decommits the pages specified in the range (in bytes).
    /// 
    /// The range must be within the size of the VMO.
    /// 
    /// The start and end addresses will be rounded down and up to page boundaries.
    /// 
    /// # Access rights
    ///
    /// The method requires the Write right. 
    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.decommit(range)
    }

    /// Resizes the VMO by giving a new size.
    /// 
    /// The VMO must be resizable.
    /// 
    /// The new size will be rounded up to page boundaries.
    /// 
    /// # Access rights
    ///
    /// The method requires the Write right. 
    pub fn resize(&self, new_size: usize) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.resize(new_size)
    }

    /// Clears the specified range by writing zeros. 
    /// 
    /// # Access rights
    ///
    /// The method requires the Write right. 
    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.clear(range) 
    }

    /// Duplicates the capability.
    /// 
    /// # Access rights
    ///
    /// The method requires the Dup right. 
    pub fn dup(&self) -> Result<Self> {
        self.check_rights(Rights::DUP)?;
        todo!()
    }

    /// Restricts the access rights given the mask.
    pub fn restrict(mut self, mask: Rights) -> Self {
        todo!()
    }

    /// Converts to a static capability.
    pub fn to_static<R1: TRights>(self) -> Result<Vmo<R1>> {
        self.check_rights(R1::BITS)?;
        todo!()
    }

    /// Returns the access rights.
    pub fn rights(&self) -> Rights {
        self.1
    }
}

impl VmIo for Vmo<Rights> {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        self.check_rights(Rights::READ)?;
        self.0.read(offset, buf)
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.write(offset, buf)
    }
}